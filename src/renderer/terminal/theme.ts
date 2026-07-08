// src/renderer/terminal/theme.ts
//
// xterm.js `ITheme` for the "green-CRT meets modern dark" pixel/retro look
// Kept as a standalone module (no logic) so
// `TerminalRegistry` and any future terminal-adjacent UI can share the exact
// same palette.
import type { ITheme } from "@xterm/xterm";

export const XTERM_THEME: ITheme = {
  background: "#12131a",
  foreground: "#c8d0e0",
  cursor: "#7CFF6B",
  cursorAccent: "#12131a",
  selectionBackground: "#2b3350",
  black: "#1b1d2a",
  red: "#ff5c6a",
  green: "#7CFF6B",
  yellow: "#ffd866",
  blue: "#6fb3ff",
  magenta: "#c792ea",
  cyan: "#5be7d6",
  white: "#c8d0e0",
  brightBlack: "#4a5170",
  brightRed: "#ff8791",
  brightGreen: "#a5ff9c",
  brightYellow: "#ffe699",
  brightBlue: "#a0cbff",
  brightMagenta: "#e0b7ff",
  brightCyan: "#8ff4e8",
  brightWhite: "#ffffff",
};
