/// The translucent rectangle drawn while box-selecting (Select tool
/// drag-from-empty-space). Mirrors `measurement-hud.ts`'s lazy-create /
/// reposition-via-inline-styles pattern.
class SelectionMarquee {
  private el: HTMLDivElement | null = null;

  private ensure(): HTMLDivElement {
    if (!this.el) {
      this.el = document.createElement("div");
      this.el.className = "selection-marquee";
      document.querySelector("#ui-root")?.appendChild(this.el);
    }
    return this.el;
  }

  /// Corners in client (viewport-relative) pixel coordinates - the same
  /// space as PointerEvent.clientX/Y, so callers can pass raw drag points
  /// with no conversion.
  show(x0: number, y0: number, x1: number, y1: number) {
    const el = this.ensure();
    el.style.left = `${Math.min(x0, x1)}px`;
    el.style.top = `${Math.min(y0, y1)}px`;
    el.style.width = `${Math.abs(x1 - x0)}px`;
    el.style.height = `${Math.abs(y1 - y0)}px`;
    el.style.display = "block";
  }

  hide() {
    if (this.el) this.el.style.display = "none";
  }
}

export const selectionMarquee = new SelectionMarquee();
