# Bundled pixel fonts

Both font files below are bundled **locally** (no CDN at runtime) and loaded
via `@font-face` in `../pixel.css`.

## Galmuri11.woff2

- Source: [quiple/galmuri](https://github.com/quiple/galmuri) release
  `v2.40.3` (`Galmuri-v2.40.3.zip`, file `Galmuri11.woff2`, unmodified).
- License: SIL Open Font License 1.1 — see `LICENSE-Galmuri11.txt`.
- Used for: terminal text (stable monospace metrics).

## DungGeunMo.woff2

- The intended font family is "DungGeunMo(둥근모)". The original 1990s DOS
  bitmap font ("둥근모꼴", public domain, by 김중태) has no official
  redistributable webfont release of its own; **Neo둥근모 (NeoDunggeunmo)**
  by Eunbin Jeong is the actively-maintained TrueType conversion of that
  exact bitmap font (see its README) and is what ships here, renamed to
  `DungGeunMo.woff2` to match this repo's `@font-face` family name.
- Source: [neodgm/neodgm](https://github.com/neodgm/neodgm) release
  `v1.601`, file `neodgm.woff2`, unmodified (only renamed).
- License: SIL Open Font License 1.1 — see `LICENSE-DungGeunMo.txt`.
- Used for: UI chrome text (buttons, bars, dialogs).

## If a different DungGeunMo build is preferred later

Replace `DungGeunMo.woff2` with the exact bytes of the desired build and
keep the filename/family name (`DungGeunMo`) so `pixel.css`'s `@font-face`
keeps working without changes. Update `LICENSE-DungGeunMo.txt` to match
whatever build is dropped in.
