#include "engine/rime_engine.h"

#include <windows.h>
#include <X11/keysym.h>

#include <mutex>
#include <system_error>
#include <utility>

#include <rime_api.h>

namespace flower::engine {
namespace {

constexpr int kRimeShiftMask = 1 << 0;
constexpr int kRimeControlMask = 1 << 2;
constexpr int kRimeAltMask = 1 << 3;

using GetRimeApi = RimeApi* (*)();

bool ToUtf8(const std::filesystem::path& path, std::string& output) {
  const std::wstring wide = path.wstring();
  if (wide.empty()) {
    output.clear();
    return true;
  }

  const int required = WideCharToMultiByte(
      CP_UTF8, WC_ERR_INVALID_CHARS, wide.data(),
      static_cast<int>(wide.size()), nullptr, 0, nullptr, nullptr);
  if (required <= 0) {
    return false;
  }

  output.resize(static_cast<std::size_t>(required));
  return WideCharToMultiByte(
             CP_UTF8, WC_ERR_INVALID_CHARS, wide.data(),
             static_cast<int>(wide.size()), output.data(), required, nullptr,
             nullptr) == required;
}

void SetWin32Error(std::string& error, const char* operation) {
  error = operation;
  error += " failed with Win32 error ";
  error += std::to_string(GetLastError());
}

bool HasDirectory(const std::filesystem::path& path, std::string& error) {
  std::error_code ec;
  if (!std::filesystem::is_directory(path, ec) || ec) {
    error = "Required directory is unavailable: ";
    error += path.u8string();
    return false;
  }
  return true;
}

void CopyRimeUpdate(RimeContext& context, EngineUpdate& update) {
  if (context.composition.preedit) {
    update.preedit = context.composition.preedit;
  }

  update.page = context.menu.page_no;
  update.last_page = context.menu.is_last_page != False;
  update.highlighted_candidate = context.menu.highlighted_candidate_index;

  if (!context.menu.candidates || context.menu.num_candidates <= 0) {
    return;
  }

  update.candidates.reserve(
      static_cast<std::size_t>(context.menu.num_candidates));
  for (int i = 0; i < context.menu.num_candidates; ++i) {
    const RimeCandidate& source = context.menu.candidates[i];
    Candidate candidate;
    if (source.text) {
      candidate.text = source.text;
    }
    if (source.comment) {
      candidate.comment = source.comment;
    }
    update.candidates.push_back(std::move(candidate));
  }
}

}

struct RimeRuntimeState {
  HMODULE module = nullptr;
  RimeApi* api = nullptr;
  std::filesystem::path runtime_directory;
  std::filesystem::path shared_data_directory;
  std::filesystem::path user_data_directory;
  bool initialized = false;

  ~RimeRuntimeState() {
    if (initialized && api && api->finalize) {
      api->finalize();
    }
    if (module) {
      FreeLibrary(module);
    }
  }
};

namespace {

std::mutex g_runtime_mutex;
std::weak_ptr<RimeRuntimeState> g_runtime_state;

bool RequiredApiAvailable(RimeApi* api) {
  return RIME_API_AVAILABLE(api, setup) &&
         RIME_API_AVAILABLE(api, initialize) &&
         RIME_API_AVAILABLE(api, finalize) &&
         RIME_API_AVAILABLE(api, create_session) &&
         RIME_API_AVAILABLE(api, destroy_session) &&
         RIME_API_AVAILABLE(api, process_key) &&
         RIME_API_AVAILABLE(api, get_commit) &&
         RIME_API_AVAILABLE(api, free_commit) &&
         RIME_API_AVAILABLE(api, get_context) &&
         RIME_API_AVAILABLE(api, free_context) &&
         RIME_API_AVAILABLE(api, clear_composition) &&
         RIME_API_AVAILABLE(api, select_candidate_on_current_page) &&
         RIME_API_AVAILABLE(api, change_page);
}

std::shared_ptr<RimeRuntimeState> LoadRuntime(const EnginePaths& paths,
                                              std::string& error) {
  std::lock_guard<std::mutex> lock(g_runtime_mutex);
  if (std::shared_ptr<RimeRuntimeState> existing = g_runtime_state.lock()) {
    if (existing->runtime_directory != paths.runtime_directory ||
        existing->shared_data_directory != paths.shared_data_directory ||
        existing->user_data_directory != paths.user_data_directory) {
      error = "librime is already initialized with different data paths.";
      return {};
    }
    return existing;
  }

  if (!HasDirectory(paths.runtime_directory, error) ||
      !HasDirectory(paths.shared_data_directory, error)) {
    return {};
  }

  std::error_code ec;
  std::filesystem::create_directories(paths.user_data_directory, ec);
  if (ec) {
    error = "Could not create Flower's per-user Rime data directory: ";
    error += ec.message();
    return {};
  }

  auto state = std::make_shared<RimeRuntimeState>();
  state->runtime_directory = paths.runtime_directory;
  state->shared_data_directory = paths.shared_data_directory;
  state->user_data_directory = paths.user_data_directory;

  const std::filesystem::path rime_dll = paths.runtime_directory / L"rime.dll";
  state->module = LoadLibraryExW(
      rime_dll.c_str(), nullptr,
      LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
  if (!state->module) {
    SetWin32Error(error, "Loading the adjacent rime.dll");
    return {};
  }

  const auto get_api = reinterpret_cast<GetRimeApi>(
      GetProcAddress(state->module, "rime_get_api"));
  if (!get_api) {
    SetWin32Error(error, "Finding rime_get_api");
    return {};
  }

  state->api = get_api();
  if (!state->api || !RequiredApiAvailable(state->api)) {
    error = "The librime runtime does not provide the required API.";
    return {};
  }

  std::string shared_data;
  std::string user_data;
  std::string prebuilt_data;
  std::string staging_data;
  if (!ToUtf8(paths.shared_data_directory, shared_data) ||
      !ToUtf8(paths.user_data_directory, user_data) ||
      !ToUtf8(paths.shared_data_directory / L"build", prebuilt_data) ||
      !ToUtf8(paths.user_data_directory / L"build", staging_data)) {
    error = "Could not encode Flower data paths as UTF-8.";
    return {};
  }

  RimeTraits traits{};
  traits.data_size = static_cast<int>(
      sizeof(RimeTraits) - sizeof(traits.data_size));
  traits.shared_data_dir = shared_data.c_str();
  traits.user_data_dir = user_data.c_str();
  traits.prebuilt_data_dir = prebuilt_data.c_str();
  traits.staging_dir = staging_data.c_str();
  traits.distribution_name = "Flower";
  traits.distribution_code_name = "flower";
  traits.distribution_version = "0.1.0";
  traits.app_name = "rime.flower";
  traits.min_log_level = 3;
  traits.log_dir = "";

  state->api->setup(&traits);
  state->api->initialize(nullptr);
  state->initialized = true;
  g_runtime_state = state;
  return state;
}

}

MappedKey MapWindowsKey(std::uint32_t virtual_key, KeyModifiers modifiers) {
  MappedKey result;
  if (modifiers.shift) {
    result.modifiers |= kRimeShiftMask;
  }
  if (modifiers.control) {
    result.modifiers |= kRimeControlMask;
  }
  if (modifiers.alt) {
    result.modifiers |= kRimeAltMask;
  }

  if (virtual_key >= 'A' && virtual_key <= 'Z') {
    result.keycode = XK_a + static_cast<int>(virtual_key - 'A');
  } else if (virtual_key >= '0' && virtual_key <= '9') {
    result.keycode = XK_0 + static_cast<int>(virtual_key - '0');
  } else {
    switch (virtual_key) {
      case VK_BACK:
        result.keycode = XK_BackSpace;
        break;
      case VK_TAB:
        result.keycode = XK_Tab;
        break;
      case VK_RETURN:
        result.keycode = XK_Return;
        break;
      case VK_ESCAPE:
        result.keycode = XK_Escape;
        break;
      case VK_SPACE:
        result.keycode = XK_space;
        break;
      case VK_PRIOR:
        result.keycode = XK_Page_Up;
        break;
      case VK_NEXT:
        result.keycode = XK_Page_Down;
        break;
      case VK_LEFT:
        result.keycode = XK_Left;
        break;
      case VK_UP:
        result.keycode = XK_Up;
        break;
      case VK_RIGHT:
        result.keycode = XK_Right;
        break;
      case VK_DOWN:
        result.keycode = XK_Down;
        break;
      default:
        return result;
    }
  }

  result.supported = true;
  return result;
}

RimeEngine::~RimeEngine() = default;
RimeEngine::RimeEngine(RimeEngine&&) noexcept = default;
RimeEngine& RimeEngine::operator=(RimeEngine&&) noexcept = default;

bool RimeEngine::Initialize(const EnginePaths& paths, std::string& error) {
  error.clear();
  state_ = LoadRuntime(paths, error);
  return static_cast<bool>(state_);
}

std::unique_ptr<RimeSession> RimeEngine::CreateSession(
    std::string& error) const {
  error.clear();
  if (!state_ || !state_->api) {
    error = "librime is not initialized.";
    return {};
  }

  const RimeSessionId id = state_->api->create_session();
  if (!id) {
    error = "librime could not create an input session.";
    return {};
  }

  if (!state_->api->select_schema(id, "flower_pinyin")) {
    state_->api->destroy_session(id);
    error = "The Flower Pinyin schema could not be loaded.";
    return {};
  }

  return std::unique_ptr<RimeSession>(
      new RimeSession(state_, static_cast<std::uintptr_t>(id)));
}

void RimeEngine::Reset() noexcept {
  state_.reset();
}

RimeSession::RimeSession(std::shared_ptr<RimeRuntimeState> state,
                         std::uintptr_t id)
    : state_(std::move(state)), id_(id) {}

RimeSession::~RimeSession() {
  if (state_ && state_->api && id_) {
    state_->api->destroy_session(static_cast<RimeSessionId>(id_));
  }
}

bool RimeSession::ProcessKey(const MappedKey& key, EngineUpdate& update,
                             std::string& error) {
  update = {};
  error.clear();
  if (!state_ || !state_->api || !id_) {
    error = "The Rime session is unavailable.";
    return false;
  }
  if (!key.supported) {
    return false;
  }
  if (!state_->api->process_key(static_cast<RimeSessionId>(id_), key.keycode,
                                key.modifiers)) {
    return false;
  }

  update.handled = true;
  return ReadUpdate(update, error);
}

bool RimeSession::SelectCandidate(std::size_t index, EngineUpdate& update,
                                  std::string& error) {
  update = {};
  error.clear();
  if (!state_ || !state_->api || !id_) {
    error = "The Rime session is unavailable.";
    return false;
  }
  if (!state_->api->select_candidate_on_current_page(
          static_cast<RimeSessionId>(id_), index)) {
    error = "librime rejected the selected candidate.";
    return false;
  }
  update.handled = true;
  return ReadUpdate(update, error);
}

bool RimeSession::ChangePage(bool backward, EngineUpdate& update,
                             std::string& error) {
  update = {};
  error.clear();
  if (!state_ || !state_->api || !id_) {
    error = "The Rime session is unavailable.";
    return false;
  }
  if (!state_->api->change_page(static_cast<RimeSessionId>(id_),
                                backward ? True : False)) {
    return false;
  }
  update.handled = true;
  return ReadUpdate(update, error);
}

void RimeSession::ClearComposition() noexcept {
  if (state_ && state_->api && id_) {
    state_->api->clear_composition(static_cast<RimeSessionId>(id_));
  }
}

bool RimeSession::ReadUpdate(EngineUpdate& update, std::string& error) {
  const RimeSessionId id = static_cast<RimeSessionId>(id_);

  RimeCommit commit{};
  commit.data_size = static_cast<int>(
      sizeof(RimeCommit) - sizeof(commit.data_size));
  if (state_->api->get_commit(id, &commit)) {
    if (commit.text) {
      update.commit = commit.text;
    }
    if (!state_->api->free_commit(&commit)) {
      error = "librime could not release committed text.";
      return false;
    }
  }

  RimeContext context{};
  context.data_size = static_cast<int>(
      sizeof(RimeContext) - sizeof(context.data_size));
  if (!state_->api->get_context(id, &context)) {
    error = "librime could not read the current composition.";
    return false;
  }
  CopyRimeUpdate(context, update);
  if (!state_->api->free_context(&context)) {
    error = "librime could not release composition data.";
    return false;
  }
  return true;
}

}
