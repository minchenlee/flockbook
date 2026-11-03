// Fake herdr data: one space with three agents, as agents() and spaces() describe them.
const program = (key, name) => ({ key, name, icon: `icons/${key}.svg`, tint: true, tile: null, color: null, shell: false, agent: true });
export function agent(address, status, extra = {}) {
  return { address, pane: `p-${address}`, space: "sp1", space_name: "app", tab: `tb-${address}`, tab_index: 1, tab_name: "1",
    status, app: "claude-code", program: program("claude", "Claude Code"), title: "", seq: 1, pinned: false, focused: false, ...extra };
}
export function space(id = "sp1", extra = {}) {
  return { id, index: 1, name: "app", named: false, folder: "/w/app", machine: null, branch: "main", active: true, status: null, tabs: [], ...extra };
}
