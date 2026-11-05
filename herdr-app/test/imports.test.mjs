// The node tests serve gpui-kit and gpui-base from one fake (stub/ui.mjs), so a name imported from the
// wrong module passes here and fails in the app ("Could not find export 'Input' in module 'gpui-kit'").
// Each name the mod imports must be one that module exports in Herdr 0.1.13 (gpui-kit.d.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

const EXPORTS = {
  "gpui-kit": "Background PathBuilder View div image list svg uniform_list",
  "gpui-base": "Accordion AccordionHeader AccordionItem AccordionPanel AccordionTrigger Avatar AvatarFallback AvatarImage "
    + "Button CalendarState Checkbox Collapsible Combobox DatePicker DockArea Editor EditorState HoverCard Input InputState "
    + "Link NumberInput OtpInput OtpState Pagination Popover Popup Progress ProgressIndicator ProgressTrack Radio RadioGroup "
    + "Scrollbar Select Slider SliderIndicator SliderState SliderThumb SliderTrack Switch Tab Table TableBody TableCaption "
    + "TableCell TableHead TableHeader TableRow Tabs TextView Textarea TextareaState Toggle ToggleGroup "
    + "VirtualListScrollHandle dock_area dock_content h_flex h_resizable h_virtual_list pagination_items resizable_panel "
    + "set_theme v_flex v_resizable v_virtual_list",
};

test("each name imported from gpui-kit or gpui-base is one that module exports", () => {
  const dir = new URL("../mods/flockbook/", import.meta.url);
  let seen = 0;
  for (const f of readdirSync(dir).filter((f) => f.endsWith(".js"))) {
    const src = readFileSync(new URL(f, dir), "utf8");
    for (const [, names, mod] of src.matchAll(/import\s*{([^}]*)}\s*from\s*"(gpui-kit|gpui-base)"/g)) {
      const has = new Set(EXPORTS[mod].split(" "));
      for (const n of names.split(",").map((s) => s.trim().split(/\s+as\s+/)[0]).filter(Boolean)) {
        assert.ok(has.has(n), `${f}: "${n}" is not exported by ${mod}`);
        seen++;
      }
    }
  }
  assert.ok(seen > 10, "the imports were read");
});
