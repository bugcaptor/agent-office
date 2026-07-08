// src/renderer/office/entities/ThinkingOverlay.ts
//
// "Thinking" indicator: a tiny speech bubble with three dots, shown above a
// character's head while they're seated, session-active, and idle (no
// pending notification). Code-drawn (no image asset), modeled on
// ExclamationOverlay.ts's structure — Pixi Graphics, pixel-art scale, a
// dt-driven animation and a setVisible-style API.
//
// Cadence (hidden ~4-7s / visible ~2-2.5s) is owned by the caller
// (`CharacterEntity`, dt-accumulator based) — this class only renders the
// bubble and animates the three dots while visible.

import { Container, Graphics } from "pixi.js";

const DOT_PERIOD_MS = 700; // one full bounce cycle per dot
const DOT_PHASE_STEP = Math.PI / 3; // stagger between successive dots (radians)
const DOT_AMPLITUDE_PX = 1.5;
const DOT_BASE_Y = -2;

export class ThinkingOverlay {
  readonly root = new Container();
  private bubble: Graphics;
  private dots: Graphics[];
  private t = 0;

  constructor() {
    this.bubble = new Graphics();
    // Small rounded speech-bubble body + a short tail pointing down toward the head.
    this.bubble.roundRect(-8, -8, 16, 9, 3).fill(0xffffff).stroke({ width: 1, color: 0x555555 });
    this.bubble.poly([-2, 1, 2, 1, 0, 4]).fill(0xffffff);
    this.root.addChild(this.bubble);

    this.dots = [-4, 0, 4].map((dx) => {
      const dot = new Graphics();
      dot.circle(0, 0, 1.2).fill(0x555555);
      dot.position.set(dx, DOT_BASE_Y);
      this.root.addChild(dot);
      return dot;
    });
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
    if (v) this.t = 0;
  }

  /** dt: ms. No-op while hidden (mirrors ExclamationOverlay: saves the sine calls, keeps phase reset on next show). */
  update(dt: number): void {
    if (!this.root.visible) return;
    this.t += dt;
    this.dots.forEach((dot, i) => {
      const phase = (this.t / DOT_PERIOD_MS) * Math.PI * 2 - i * DOT_PHASE_STEP;
      dot.y = DOT_BASE_Y + Math.sin(phase) * DOT_AMPLITUDE_PX;
    });
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}
