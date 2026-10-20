#include "tsf/text_service.h"

#include <windows.h>

#include <atomic>
#include <new>

namespace {

std::atomic<ULONG> g_object_count{0};
std::atomic<ULONG> g_server_lock_count{0};

class ClassFactory final : public IClassFactory {
 public:
  ClassFactory() {
    flower::tsf::ObjectCreated();
  }

  ~ClassFactory() {
    flower::tsf::ObjectDestroyed();
  }

  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
    if (!object) {
      return E_POINTER;
    }
    *object = nullptr;
    if (iid != IID_IUnknown && iid != IID_IClassFactory) {
      return E_NOINTERFACE;
    }
    *object = static_cast<IClassFactory*>(this);
    AddRef();
    return S_OK;
  }

  ULONG STDMETHODCALLTYPE AddRef() override {
    return ++reference_count_;
  }

  ULONG STDMETHODCALLTYPE Release() override {
    const ULONG remaining = --reference_count_;
    if (remaining == 0) {
      delete this;
    }
    return remaining;
  }

  HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown* outer, REFIID iid,
                                           void** object) override {
    if (!object) {
      return E_POINTER;
    }
    *object = nullptr;
    if (outer) {
      return CLASS_E_NOAGGREGATION;
    }

    auto* service = new (std::nothrow) flower::tsf::TextService();
    if (!service) {
      return E_OUTOFMEMORY;
    }
    const HRESULT result = service->QueryInterface(iid, object);
    service->Release();
    return result;
  }

  HRESULT STDMETHODCALLTYPE LockServer(BOOL lock) override {
    if (lock) {
      flower::tsf::ServerLockAcquired();
      return S_OK;
    }
    return flower::tsf::ServerLockReleased();
  }

 private:
  std::atomic<ULONG> reference_count_{1};
};

}

namespace flower::tsf {

void ObjectCreated() noexcept {
  ++g_object_count;
}

void ObjectDestroyed() noexcept {
  --g_object_count;
}

void ServerLockAcquired() noexcept {
  ++g_server_lock_count;
}

HRESULT ServerLockReleased() noexcept {
  ULONG current = g_server_lock_count.load();
  while (current > 0) {
    if (g_server_lock_count.compare_exchange_weak(current, current - 1)) {
      return S_OK;
    }
  }
  return E_UNEXPECTED;
}

bool CanUnload() noexcept {
  return g_object_count == 0 && g_server_lock_count == 0;
}

}

extern "C" HRESULT STDAPICALLTYPE DllGetClassObject(
    REFCLSID class_id, REFIID iid, void** object) {
  if (!object) {
    return E_POINTER;
  }
  *object = nullptr;
  if (class_id != flower::tsf::kFlowerTextServiceClsid) {
    return CLASS_E_CLASSNOTAVAILABLE;
  }

  auto* factory = new (std::nothrow) ClassFactory();
  if (!factory) {
    return E_OUTOFMEMORY;
  }
  const HRESULT result = factory->QueryInterface(iid, object);
  factory->Release();
  return result;
}

extern "C" HRESULT STDAPICALLTYPE DllCanUnloadNow() {
  return flower::tsf::CanUnload() ? S_OK : S_FALSE;
}
