import HerdrKit
import SwiftUI

struct InboxView: View {
    @Environment(AppModel.self) private var model
    @State private var pick: String?

    var body: some View {
        let items = model.allBlocked
        if items.isEmpty {
            // Whole content area, no panel: centered.
            ContentUnavailableView {
                Label { Text("All clear") } icon: { Image(systemName: "checkmark.circle").foregroundStyle(.green) }
            } description: { Text("No agent is waiting for you.") }
        } else {
            HStack(spacing: 0) {
                list(items).frame(width: 340)
                Divider()
                detail(items).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    private func list(_ items: [(Workspace, Tab)]) -> some View {
        let all = model.store.workspaces.flatMap { ws in ws.tabs.map { (ws, $0) } }
        let working = all.filter { $0.1.status == .working }
        let resting = all.filter { $0.1.status == .done || $0.1.status == .idle }.count
        let current = items.first(where: { $0.1.id == pick })?.1.id ?? items.first?.1.id
        return OptionalScroll {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 8) {
                    Text("Needs you").font(.title3.weight(.semibold))
                    Text("\(items.count)").foregroundStyle(.tertiary)
                    Spacer()
                }.padding(.horizontal, 16).padding(.top, 44).padding(.bottom, 8)
                ForEach(items, id: \.1.id) { ws, t in
                    let sel = t.id == current
                    Button { pick = t.id } label: {
                        HStack(spacing: 10) {
                            ZStack(alignment: .bottomTrailing) {
                                Image(systemName: role(for: t.primaryAgent?.name ?? "").symbol).font(.body)
                                    .frame(width: 30, height: 30)
                                    .background(sel ? Color.white.opacity(0.22) : Color.primary.opacity(0.08), in: Circle())
                                StatusGlyph(status: .blocked, size: 13).background(Circle().fill(sel ? Color.accentColor : Color(nsColor: .windowBackgroundColor)))
                            }
                            VStack(alignment: .leading, spacing: 1) {
                                Text(t.primaryAgent?.name ?? t.label).fontWeight(.semibold).lineLimit(1)
                                Text("\(Monogram.displayName(ws.label)) · \(t.label)").font(.caption)
                                    .foregroundStyle(sel ? Color.white.opacity(0.8) : Color.secondary).lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .foregroundStyle(sel ? Color.white : Color.primary)
                        .padding(.horizontal, 12).padding(.vertical, 9)
                        .background(sel ? Color.accentColor : .clear, in: RoundedRectangle(cornerRadius: Radius.control))
                        .contentShape(Rectangle())
                    }.buttonStyle(.plain).padding(.horizontal, 8)
                }
                if !working.isEmpty {
                    Text("WORKING · NO ACTION NEEDED").font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                        .padding(.horizontal, 16).padding(.top, 18).padding(.bottom, 4)
                    ForEach(working, id: \.1.id) { ws, t in
                        Button { model.select(tab: t, in: ws) } label: {
                            HStack(spacing: 10) {
                                StatusGlyph(status: .working, size: 14)
                                VStack(alignment: .leading, spacing: 0) {
                                    Text(t.primaryAgent?.name ?? t.label).font(.callout).lineLimit(1)
                                    Text(t.label).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                }
                                Spacer(minLength: 0)
                            }.padding(.horizontal, 16).padding(.vertical, 4).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                    }
                }
                if resting > 0 {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                        Text("\(resting) agents resting").foregroundStyle(.secondary)
                    }
                    .font(.callout).padding(.horizontal, 12).padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: Radius.card))
                    .padding(.horizontal, 16).padding(.top, 12)
                }
                Spacer(minLength: 0)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)   // on the window surface like every screen; the divider separates
    }

    @ViewBuilder private func detail(_ items: [(Workspace, Tab)]) -> some View {
        if let sel = items.first(where: { $0.1.id == pick }) ?? items.first {
            let (ws, t) = sel
            let agent = t.primaryAgent
            OptionalScroll { detailBody(ws, t, agent) }   // the whole detail width scrolls; the content stays in the column
                .task(id: t.id) { await model.watchPrompts(tabId: t.id) }
        } else {
            Color.clear
        }
    }

    @ViewBuilder private func detailBody(_ ws: Workspace, _ t: Tab, _ agent: Agent?) -> some View {
        // The card's text and its buttons belong to one pane: the blocked one whose screen shows the prompt (kept while
        // its answer is on the way, with the buttons off).
        let cardPane = PermissionGate.cardPane(t.panes, promptVisible: model.promptVisible.union(model.approvals.inFlight))
        let request: PermissionRequest? = cardPane.flatMap { model.approvalRequest($0) }
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 14) {
                ZStack(alignment: .bottomTrailing) {
                    Image(systemName: role(for: agent?.name ?? "").symbol)
                        .font(.title)
                        .frame(width: 56, height: 56)
                        .background(Color.primary.opacity(0.08), in: Circle())
                        .overlay(Circle().stroke(Color.orange, lineWidth: 2))
                    StatusGlyph(status: .blocked, size: 18)
                }
                VStack(alignment: .leading, spacing: 3) {
                    Text(agent?.name ?? t.label).font(.title.bold())
                    Text("\(ws.label) · \(t.label) · Needs you").foregroundStyle(.secondary)
                }
            }
            if let cardPane, let request, request.state == .pending {
                ApprovalCard(request: request, agentName: cardPane.agent?.name ?? "", canRespond: model.promptVisible.contains(cardPane.id),
                             onAllow: { model.approve(pane: cardPane, allow: true, bound: request) },
                             onDeny: { model.approve(pane: cardPane, allow: false, bound: request) },
                             onOpen: { model.select(tab: t, in: ws); model.showTerminal = true })
                    .frame(maxWidth: 560)
            } else {
                Text("The request text is not available from the session log. Open the tab to see it.")
                    .foregroundStyle(.secondary)
                Button("Open tab") { model.select(tab: t, in: ws) }.buttonStyle(.borderedProminent)
            }
        }
        .padding(32).padding(.top, 16).screenColumn("needs-you")
    }
}

struct OverviewView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("overviewGroup", store: AppDefaults.store) private var group = "status"
    @State private var now = Date()

    private func rank(_ s: AgentStatus) -> Int { s == .blocked ? 0 : s == .working ? 1 : s == .done ? 2 : 3 }

    var body: some View {
        let all = model.store.workspaces.flatMap { ws in ws.tabs.map { (ws, $0) } }
        let groups = OverviewGroups(all) { $0.1 }
        let blocked = groups.needsYou, working = groups.working, calm = groups.resting
        OptionalScroll {
            VStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(greeting).font(.largeTitle.bold())
                        Text("\(all.count) tabs · \(working.count) working · \(blocked.count) need you")
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Picker("Group by", selection: $group) {
                        Text("Status").tag("status")
                        Text("Workspace").tag("workspace")
                    }
                    .pickerStyle(.menu).fixedSize()
                }.padding(.top, 36)
                if group == "workspace" {
                    ForEach(model.store.workspaces.filter { !$0.tabs.isEmpty }.sorted { a, b in
                        let ka = a.blockedCount > 0 ? 0 : a.workingCount > 0 ? 1 : 2
                        let kb = b.blockedCount > 0 ? 0 : b.workingCount > 0 ? 1 : 2
                        return ka < kb
                    }, id: \.id) { ws in
                        workspaceSection(ws)
                    }
                } else {
                    section("Needs you", blocked)
                    section("Working", working)
                    section("Done and resting", calm)
                }
            }
            .padding(.horizontal, 36).padding(.bottom, 36)
            .screenColumn("overview")   // centred: a wide window does not leave the grid against the left edge
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .task {   // the card ages: the logs' times (each looked at once per 30 s at most) and the clock they count from
            while !Task.isCancelled {
                await model.store.refreshLogTimes()
                now = Date()
                try? await Task.sleep(for: .seconds(15))
            }
        }
    }

    private var greeting: String {
        let h = Calendar.current.component(.hour, from: Date())
        return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening"
    }

    private func workspaceSection(_ ws: Workspace) -> some View {
        var parts: [String] = []
        if ws.blockedCount > 0 { parts.append("\(ws.blockedCount) need you") }
        if ws.workingCount > 0 { parts.append("\(ws.workingCount) working") }
        let calm = ws.tabs.count - ws.blockedCount - ws.workingCount
        if calm > 0 { parts.append("\(calm) resting") }
        let rows = ws.tabs.sorted { rank($0.status) < rank($1.status) }.map { (ws, $0) }
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 9) {
                Monogram(text: ws.label)
                Text(Monogram.displayName(ws.label)).font(.headline)
                Text(parts.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary)
            }
            cards(rows)
        }
    }

    @ViewBuilder private func section(_ title: String, _ rows: [(Workspace, Tab)]) -> some View {
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text(title).font(.headline)
                cards(rows)
            }
        }
    }

    private func cards(_ rows: [(Workspace, Tab)]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 10)], spacing: 10) {
            ForEach(rows, id: \.1.id) { ws, t in
                Button { model.select(tab: t, in: ws) } label: {
                    HStack(spacing: 10) {
                        StatusGlyph(status: t.status, size: 16)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(t.primaryAgent?.name ?? t.label).font(.headline).lineLimit(1)
                                    .background { OverviewProbe.reader(t.id, \.name) }
                                Spacer(minLength: 0)
                                // How long ago its agent last wrote its log; none when no log was found (a shell tab).
                                if let at = model.store.lastActivity(of: t, now: now) {
                                    Text(LogAge.label(at, now: now)).font(.caption).foregroundStyle(.secondary).monospacedDigit()
                                        .background { OverviewProbe.reader(t.id, \.age, LogAge.label(at, now: now)) }
                                }
                            }
                            Text("\(ws.label) · \(t.label)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    .padding(12)
                    .background { OverviewProbe.reader(t.id, \.card) }
                    .background { GlassReader { RoundedRectangle(cornerRadius: Radius.card).fill($0.card) } }
                    .overlay(RoundedRectangle(cornerRadius: Radius.card).stroke(t.status == .blocked ? Color.orange.opacity(0.5) : Color.primary.opacity(0.09)))
                }.buttonStyle(.plain)
            }
        }
    }
}

/// Where the Overview cards drew their parts, by tab id, in window coordinates (checks).
@MainActor enum OverviewProbe {
    struct Card { var name = CGRect.zero, age = CGRect.zero, card = CGRect.zero, ageText: String? }
    static var cards: [String: Card] = [:]
    static func reader(_ id: String, _ part: WritableKeyPath<Card, CGRect>, _ text: String? = nil) -> some View {
        GeometryReader { g in
            Color.clear.onAppear { cards[id, default: Card()][keyPath: part] = g.frame(in: .global); if let text { cards[id]?.ageText = text } }
                .onChange(of: g.frame(in: .global)) { cards[id, default: Card()][keyPath: part] = $1 }
        }
    }
}

/// ScrollView in the real window; plain content in snapshot mode (off-screen NSScrollView does not paint).
struct OptionalScroll<Content: View>: View {
    @Environment(\.flatGlass) private var flat
    @ViewBuilder var content: Content
    var body: some View {
        if flat { content } else { ScrollView { content }.defaultScrollAnchor(.top) }
    }
}
