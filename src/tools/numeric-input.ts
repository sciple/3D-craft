/// Accumulates keystrokes into one or more typed numbers while a tool's
/// drag/click gesture is in progress, so a dimension can be entered exactly
/// (SketchUp's "Measurements box" typed-input convention) instead of only
/// ever being read off the mouse. Deliberately keyboard-driven with no
/// focused DOM input: draw/transform tools already own pointer + keyboard
/// input for the viewport, and routing through an actual `<input>` would
/// steal focus from the canvas mid-gesture.
///
/// Callers own Enter/Escape semantics themselves (a fresh click vs. ending a
/// drag vs. closing a polygon loop all differ per tool) - this class only
/// owns the text buffer and its parsing.
export class NumericBuffer {
  private text = "";

  get isEmpty(): boolean {
    return this.text.length === 0;
  }

  get display(): string {
    return this.text;
  }

  /// Feeds one keydown event in. Returns true if the key was consumed (a
  /// digit, '.', '-', or a ','/'x' segment separator, or Backspace on a
  /// non-empty buffer) - callers should re-render their live preview/HUD
  /// from `values()` when this returns true. Enter/Escape are intentionally
  /// not handled here.
  type(e: KeyboardEvent): boolean {
    if (e.key === "Backspace") {
      if (this.isEmpty) return false;
      this.text = this.text.slice(0, -1);
      return true;
    }
    if (/^[0-9.,x-]$/i.test(e.key)) {
      this.text += e.key;
      return true;
    }
    return false;
  }

  /// Parses the buffer into its numeric segments, split on ',' or 'x'/'X'
  /// (e.g. a rectangle's "20,10" or "20x10"). Non-numeric/empty segments
  /// are dropped rather than producing NaN entries.
  values(): number[] {
    return this.text
      .split(/[,x]/i)
      .map((s) => Number.parseFloat(s))
      .filter((n) => Number.isFinite(n));
  }

  clear() {
    this.text = "";
  }
}
