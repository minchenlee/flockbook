// Flockbook for herdr-app: one mod, four views. The layout picks the view with props.view
// ("top", "side", "chat", "needs", "overview"). One folder, because a mod cannot import another mod's files.
import { View, div } from "gpui-kit";
import { theme, props } from "herdr";
import { TopView } from "./top.js";
import { SideView } from "./side.js";
import { ChatView } from "./chat.js";
import { NeedsView } from "./needs.js";
import { TermBar } from "./termbar.js";
import { OverviewView } from "./overview.js";
import { MenuView } from "./menu.js";

const VIEWS = { top: TopView, side: SideView, chat: ChatView, needs: NeedsView, termbar: TermBar, overview: OverviewView, menu: MenuView };

export default class Flockbook extends View {
  // The block's props come from props(); init's first argument is not them.
  init(_props, cx) {
    const name = props()?.view;
    const Impl = VIEWS[name];
    this.name = name;
    this.view = Impl ? new Impl(cx) : null;
  }

  render(cx) {
    if (this.view) return this.view.render(cx);
    const t = theme();
    return div().size_full().p(12).text_color(t.muted)
      .child(`Flockbook: unknown view "${this.name}". Use one of ${Object.keys(VIEWS).join(", ")}.`);
  }
}
