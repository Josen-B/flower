#pragma once

#include <windows.h>
#include <msctf.h>

#include <atomic>

namespace flower::tsf {

inline constexpr CLSID kFlowerTextServiceClsid = {
    0x3d5b7cb1,
    0x96cb,
    0x4f44,
    {0xa7, 0x92, 0x64, 0x30, 0x4f, 0x6e, 0x74, 0x26}};

class TextService final : public ITfTextInputProcessor,
                          public ITfKeyEventSink {
 public:
  TextService();
  ~TextService();

  TextService(const TextService&) = delete;
  TextService& operator=(const TextService&) = delete;

  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override;
  ULONG STDMETHODCALLTYPE AddRef() override;
  ULONG STDMETHODCALLTYPE Release() override;

  HRESULT STDMETHODCALLTYPE Activate(ITfThreadMgr* thread_manager,
                                     TfClientId client_id) override;
  HRESULT STDMETHODCALLTYPE Deactivate() override;

  HRESULT STDMETHODCALLTYPE OnSetFocus(BOOL foreground) override;
  HRESULT STDMETHODCALLTYPE OnTestKeyDown(ITfContext* context, WPARAM key,
                                          LPARAM key_data,
                                          BOOL* eaten) override;
  HRESULT STDMETHODCALLTYPE OnTestKeyUp(ITfContext* context, WPARAM key,
                                        LPARAM key_data,
                                        BOOL* eaten) override;
  HRESULT STDMETHODCALLTYPE OnKeyDown(ITfContext* context, WPARAM key,
                                      LPARAM key_data,
                                      BOOL* eaten) override;
  HRESULT STDMETHODCALLTYPE OnKeyUp(ITfContext* context, WPARAM key,
                                    LPARAM key_data,
                                    BOOL* eaten) override;
  HRESULT STDMETHODCALLTYPE OnPreservedKey(ITfContext* context,
                                           REFGUID key,
                                           BOOL* eaten) override;

 private:
  std::atomic<ULONG> reference_count_{1};
  ITfKeystrokeMgr* keystroke_manager_ = nullptr;
  TfClientId client_id_ = TF_CLIENTID_NULL;
};

void ObjectCreated() noexcept;
void ObjectDestroyed() noexcept;
void ServerLockAcquired() noexcept;
HRESULT ServerLockReleased() noexcept;
bool CanUnload() noexcept;

}
