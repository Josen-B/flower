#include "tsf/text_service.h"

namespace flower::tsf {

TextService::TextService() {
  ObjectCreated();
}

TextService::~TextService() {
  ObjectDestroyed();
}

HRESULT STDMETHODCALLTYPE TextService::QueryInterface(REFIID iid,
                                                      void** object) {
  if (!object) {
    return E_POINTER;
  }
  *object = nullptr;

  if (iid == IID_IUnknown || iid == IID_ITfTextInputProcessor) {
    *object = static_cast<ITfTextInputProcessor*>(this);
  } else if (iid == IID_ITfKeyEventSink) {
    *object = static_cast<ITfKeyEventSink*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

ULONG STDMETHODCALLTYPE TextService::AddRef() {
  return ++reference_count_;
}

ULONG STDMETHODCALLTYPE TextService::Release() {
  const ULONG remaining = --reference_count_;
  if (remaining == 0) {
    delete this;
  }
  return remaining;
}

HRESULT STDMETHODCALLTYPE TextService::Activate(ITfThreadMgr* thread_manager,
                                                TfClientId client_id) {
  if (!thread_manager) {
    return E_INVALIDARG;
  }
  if (keystroke_manager_) {
    return E_UNEXPECTED;
  }

  ITfKeystrokeMgr* keystroke_manager = nullptr;
  const HRESULT query_result = thread_manager->QueryInterface(
      IID_ITfKeystrokeMgr,
      reinterpret_cast<void**>(&keystroke_manager));
  if (FAILED(query_result)) {
    return query_result;
  }

  const HRESULT advise_result =
      keystroke_manager->AdviseKeyEventSink(client_id, this, TRUE);
  if (FAILED(advise_result)) {
    keystroke_manager->Release();
    return advise_result;
  }

  keystroke_manager_ = keystroke_manager;
  client_id_ = client_id;
  AddRef();
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::Deactivate() {
  if (!keystroke_manager_) {
    return S_OK;
  }

  ITfKeystrokeMgr* keystroke_manager = keystroke_manager_;
  const HRESULT result = keystroke_manager->UnadviseKeyEventSink(client_id_);
  if (FAILED(result)) {
    return result;
  }

  keystroke_manager_ = nullptr;
  client_id_ = TF_CLIENTID_NULL;
  keystroke_manager->Release();
  Release();
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnSetFocus(BOOL) {
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnTestKeyDown(ITfContext*, WPARAM,
                                                     LPARAM, BOOL* eaten) {
  if (!eaten) {
    return E_POINTER;
  }
  *eaten = FALSE;
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnTestKeyUp(ITfContext*, WPARAM,
                                                   LPARAM, BOOL* eaten) {
  if (!eaten) {
    return E_POINTER;
  }
  *eaten = FALSE;
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnKeyDown(ITfContext*, WPARAM, LPARAM,
                                                 BOOL* eaten) {
  if (!eaten) {
    return E_POINTER;
  }
  *eaten = FALSE;
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnKeyUp(ITfContext*, WPARAM, LPARAM,
                                               BOOL* eaten) {
  if (!eaten) {
    return E_POINTER;
  }
  *eaten = FALSE;
  return S_OK;
}

HRESULT STDMETHODCALLTYPE TextService::OnPreservedKey(ITfContext*, REFGUID,
                                                      BOOL* eaten) {
  if (!eaten) {
    return E_POINTER;
  }
  *eaten = FALSE;
  return S_OK;
}

}
