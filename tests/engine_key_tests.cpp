#include "engine/rime_engine.h"

#include <windows.h>
#include <X11/keysym.h>

namespace {

bool Check(bool condition) {
  return condition;
}

}

int main() {
  using flower::engine::KeyModifiers;
  using flower::engine::MapWindowsKey;

  if (!Check(MapWindowsKey('A', {}).supported) ||
      !Check(MapWindowsKey('A', {}).keycode == XK_a) ||
      !Check(MapWindowsKey('Z', {}).keycode == XK_z)) {
    return 1;
  }

  const auto shifted = MapWindowsKey('B', KeyModifiers{true, false, false});
  if (!Check(shifted.supported) || !Check(shifted.keycode == XK_b) ||
      !Check(shifted.modifiers == 1)) {
    return 2;
  }

  const auto number = MapWindowsKey('7', {});
  if (!Check(number.supported) || !Check(number.keycode == XK_7)) {
    return 3;
  }

  const auto backspace = MapWindowsKey(VK_BACK, {});
  const auto page_up = MapWindowsKey(VK_PRIOR, {});
  if (!Check(backspace.supported) ||
      !Check(backspace.keycode == XK_BackSpace) ||
      !Check(page_up.supported) || !Check(page_up.keycode == XK_Page_Up)) {
    return 4;
  }

  const auto control = MapWindowsKey('C', KeyModifiers{false, true, false});
  if (!Check(control.supported) || !Check(control.keycode == XK_c) ||
      !Check(control.modifiers == (1 << 2))) {
    return 5;
  }

  if (!Check(!MapWindowsKey(VK_F1, {}).supported)) {
    return 6;
  }

  return 0;
}
