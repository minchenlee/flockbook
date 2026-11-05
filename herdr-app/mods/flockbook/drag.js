// Press-and-drag to reorder a row of items: the top bar's tabs (along x) and the sidebar's workspaces
// (along y). One drag at a time per list, kept at module level so every copy of a view draws from it.
// A press starts it; it counts as a drag once the pointer has moved DRAG_PX, and until then the
// release is a plain click. Herdr's mouse moves carry no button state and arrive only over the list,
// so a release outside it is never seen: leaving the list, another button, or a press elsewhere
// cancels instead.
export const DRAG_PX = 4;

// Where a drop puts the dragged item: its position in `items` after the move, or -1 when it would
// stay where it is. `d.target`: { id, after } — before or after that item.
export function dropPlace(items, d) {
  const from = items.findIndex((x) => x.id === d?.id), on = items.findIndex((x) => x.id === d?.target?.id);
  if (from < 0 || on < 0) return -1;
  const slot = on + (d.target.after ? 1 : 0);
  const to = slot > from ? slot - 1 : slot;
  return to === from ? -1 : to;
}

// axis "x": the item's halves are left and right; "y": top and bottom.
export function dragger(axis) {
  const size = axis === "x" ? "width" : "height";
  let d = null;   // { id, at, moved, target }
  return {
    // The drag being shown (moved past DRAG_PX), or null.
    get shown() { return d?.moved ? d : null; },
    press(id, e) { d = { id, at: e?.position?.[axis] ?? 0, moved: false, target: null }; },
    // The pointer over item `id`; true when the place changed (redraw).
    over(id, e) {
      if (!d || (!d.moved && Math.abs((e?.position?.[axis] ?? 0) - d.at) < DRAG_PX)) return false;
      // The half of this item under the pointer: its painted box, so a scrolled list counts too.
      const after = (e?.local_position?.[axis] ?? 0) >= (e?.bounds?.[size] ?? 0) / 2;
      const was = d.moved ? d.target : null;
      d.moved = true;
      d.target = { id, after };
      return was?.id !== id || was?.after !== after;
    },
    // Ends the press; redraws when a drag was shown.
    cancel(cx) { const shown = d?.moved; d = null; if (shown) cx?.notify?.(); },
    // The release: the drag to drop (moved), else null. Ends the press either way.
    release(cx) { const was = d; d = null; if (!was?.moved) return null; cx?.notify?.(); return was; },
  };
}
