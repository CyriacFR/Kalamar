// Maps browser input events to the runtime's key names (docs/controls.md, "Remapping").

const CODE_TO_NAME: Record<string, string> = {
  Space: "space", Tab: "tab", Enter: "enter", Backspace: "backspace", ShiftLeft: "lshift", ShiftRight: "rshift",
  ControlLeft: "lctrl", ControlRight: "rctrl", AltLeft: "lalt", CapsLock: "capslock", Insert: "insert", Delete: "delete",
  Home: "home", End: "end", PageUp: "pageup", PageDown: "pagedown", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left",
  ArrowRight: "right", NumpadAdd: "num_add", NumpadSubtract: "num_sub", NumpadMultiply: "num_mul", NumpadDivide: "num_div",
  NumpadDecimal: "num_decimal", Minus: "minus", Equal: "equals", BracketLeft: "lbracket", BracketRight: "rbracket",
  Semicolon: "semicolon", Quote: "apostrophe", Comma: "comma", Period: "period", Slash: "slash", Backslash: "backslash",
  Backquote: "grave",
};

export function keyNameFromEvent(e: KeyboardEvent): string | null {
  const c = e.code;
  let m = /^Key([A-Z])$/.exec(c);
  if (m) return m[1].toLowerCase();
  m = /^Digit([0-9])$/.exec(c);
  if (m) return m[1];
  m = /^Numpad([0-9])$/.exec(c);
  if (m) return `num${m[1]}`;
  m = /^F([0-9]{1,2})$/.exec(c);
  if (m) return `f${m[1]}`;
  return CODE_TO_NAME[c] ?? null;
}

export function mouseNameFromEvent(e: MouseEvent): string | null {
  return ["mouse_l", "mouse_m", "mouse_r", "mouse_x1", "mouse_x2"][e.button] ?? null;
}
