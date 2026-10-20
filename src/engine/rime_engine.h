#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <memory>
#include <string>
#include <vector>

namespace flower::engine {

struct EnginePaths {
  std::filesystem::path runtime_directory;
  std::filesystem::path shared_data_directory;
  std::filesystem::path user_data_directory;
};

struct KeyModifiers {
  bool shift = false;
  bool control = false;
  bool alt = false;
};

struct MappedKey {
  int keycode = 0;
  int modifiers = 0;
  bool supported = false;
};

struct RimeRuntimeState;

struct Candidate {
  std::string text;
  std::string comment;
};

struct EngineUpdate {
  bool handled = false;
  std::string preedit;
  std::string commit;
  std::vector<Candidate> candidates;
  int highlighted_candidate = -1;
  int page = 0;
  bool last_page = true;
};

MappedKey MapWindowsKey(std::uint32_t virtual_key, KeyModifiers modifiers);

class RimeSession;

class RimeEngine final {
 public:
  RimeEngine() = default;
  ~RimeEngine();

  RimeEngine(const RimeEngine&) = delete;
  RimeEngine& operator=(const RimeEngine&) = delete;
  RimeEngine(RimeEngine&&) noexcept;
  RimeEngine& operator=(RimeEngine&&) noexcept;

  bool Initialize(const EnginePaths& paths, std::string& error);
  std::unique_ptr<RimeSession> CreateSession(std::string& error) const;
  void Reset() noexcept;

 private:
  std::shared_ptr<RimeRuntimeState> state_;

   friend class RimeSession;
};

class RimeSession final {
 public:
  ~RimeSession();

  RimeSession(const RimeSession&) = delete;
  RimeSession& operator=(const RimeSession&) = delete;

  bool ProcessKey(const MappedKey& key, EngineUpdate& update,
                  std::string& error);
  bool SelectCandidate(std::size_t index, EngineUpdate& update,
                       std::string& error);
  bool ChangePage(bool backward, EngineUpdate& update, std::string& error);
  void ClearComposition() noexcept;

 private:
  RimeSession(std::shared_ptr<RimeRuntimeState> state, std::uintptr_t id);

  bool ReadUpdate(EngineUpdate& update, std::string& error);

  std::shared_ptr<RimeRuntimeState> state_;
  std::uintptr_t id_ = 0;

  friend class RimeEngine;
};

}
