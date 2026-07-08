// src/renderer/office/entities/ExclamationOverlay.ts
//
// Pending-notification indicator: a small yellow "!" badge, code-drawn
// (no image asset) with a sine-wave vertical bounce while visible.

import { Container, Graphics } from "pixi.js";

const BOUNCE_PERIOD_MS = 600;
const BOUNCE_AMPLITUDE_PX = 2;

export class ExclamationOverlay {
  readonly root = new Container();
  private mark: Graphics;
  private t = 0;

  constructor() {
    this.mark = new Graphics();
    // Yellow circular badge + "!" glyph (pixel-art-ish: two filled rects).
    this.mark.circle(0, 0, 6).fill(0xffcc33).stroke({ width: 1, color: 0x8a5a00 });
    this.mark.rect(-1, -4, 2, 5).fill(0x3a2600); // exclamation body
    this.mark.rect(-1, 3, 2, 2).fill(0x3a2600); // dot
    this.root.addChild(this.mark);
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
    if (v) this.t = 0;
  }

  /** dt: ms. No-op while hidden (saves the sine call, and keeps phase reset on next show). */
  update(dt: number): void {
    if (!this.root.visible) return;
    this.t += dt;
    this.mark.y = Math.round(Math.sin((this.t / BOUNCE_PERIOD_MS) * Math.PI * 2) * BOUNCE_AMPLITUDE_PX - BOUNCE_AMPLITUDE_PX);
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}
