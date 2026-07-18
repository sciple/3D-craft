/// A small read-only HUD chip (bottom-center of the viewport) showing the
/// active tool's live measurement - drag distance, rectangle dimensions,
/// rotation angle, etc. - and, while the user is typing a precise value,
/// that typed value instead. Mirrors SketchUp's "Measurements" box, minus
/// the box itself being a focusable input: typed digits are fed in by the
/// active tool's own keydown handler (see `tools/numeric-input.ts`), not by
/// focusing this element.
class MeasurementHud {
  private el: HTMLDivElement | null = null;

  private ensure(): HTMLDivElement {
    if (!this.el) {
      this.el = document.createElement("div");
      this.el.className = "measurement-hud";
      document.querySelector("#ui-root")?.appendChild(this.el);
    }
    return this.el;
  }

  /// `typed`, when non-empty, overrides `liveValue` in the display so the
  /// user sees exactly what they're typing rather than the mouse-driven
  /// reading it's about to replace.
  show(label: string, liveValue: string, typed: string | null) {
    const el = this.ensure();
    const showingTyped = !!typed;
    el.textContent = `${label}: ${showingTyped ? typed : liveValue}`;
    el.classList.toggle("typed", showingTyped);
    el.style.display = "block";
  }

  hide() {
    if (this.el) this.el.style.display = "none";
  }
}

export const measurementHud = new MeasurementHud();
