import HerdrKit
import SwiftUI

struct TabScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.underSettings) private var underSettings

    var body: some View {
        VStack(spacing: 0) {
            if let tab = model.tab {
                PaneStack(tab: tab)
            } else {
                ContentUnavailableView {
                    Label("No Tabs", systemImage: "rectangle.dashed")
                } description: {
                    Text("This workspace has no tabs.")
                } actions: {
                    Button("New Tab") { model.newTab() }.disabled(!model.writesEnabled)
                }
            }
        }
        .overlay(alignment: .top) {
            // Under the top safe area (toolbar), label colour on material: readable in light and dark. Stays ~8 s or until
            // the next notice replaces it; VoiceOver reads it.
            if let n = model.notice {
                NoticeCapsule(text: n)
                    .padding(.top, 36)   // clear of a pane label row (28)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .task(id: n) {
                        AccessibilityNotification.Announcement(n).post()
                        try? await Task.sleep(for: .seconds(8))
                        withAnimation(.easeOut(duration: 0.2)) { model.notice = nil }
                    }
            }
        }
        .animation(.easeOut(duration: 0.2), value: model.notice)
        // Not over the Settings page (a late approval result asks for the terminal): they wait for Back.
        .sheet(isPresented: Binding(get: { model.showTerminal && !underSettings }, set: { model.showTerminal = $0 })) { TerminalSheet() }
        .confirmationDialog("Close this pane?", isPresented: Binding(get: { model.closingPane != nil && !underSettings }, set: { if !$0 { model.closingPane = nil } }),
                            titleVisibility: .visible) {
            Button("Close pane", role: .destructive) { if let p = model.closingPane { model.closePane(p) } }
            Button("Cancel", role: .cancel) {}
        } message: { Text("An agent is running in this pane. Closing it ends the agent.") }
    }
}

struct PaneStack: View {
    @Environment(AppModel.self) private var model
    let tab: Tab

    var body: some View {
        let layout = model.splitDirection == "right" ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
        let zoomed = model.zoomedPaneID(in: tab)
        let panes = zoomed.map { id in tab.panes.filter { $0.id == id } } ?? tab.panes
        layout {
            ForEach(panes) { pane in
                PaneView(pane: pane, showLabel: tab.isSplit,
                         focused: !tab.isSplit || model.focusedPaneID(in: tab) == pane.id,
                         zoomed: zoomed == pane.id, tab: tab,
                         onFocus: { model.focus(pane, in: tab) })
                    .onTapGesture { model.focus(pane, in: tab) }
                    .frame(maxHeight: pane.agent == nil && tab.isSplit && zoomed == nil && model.splitDirection != "right" ? 212 : .infinity)
                if pane.id != panes.last?.id { Divider() }
            }
        }
    }
}

struct PaneView: View {
    @Environment(AppModel.self) private var model
    let pane: Pane
    let showLabel: Bool
    let focused: Bool
    var zoomed = false
    var tab: Tab? = nil
    var onFocus: () -> Void = {}
    @State private var branch: String?
    @Environment(\.flatGlass) private var flat

    @State private var focusRequest = 0
    /// Claude's open question picker, read from the screen (Claude logs the question only after the answer).
    @State private var screenQuestion: QuestionRequest?
    /// Questions answered from Desk: no card while the log or the screen catches up.
    @State private var answeredQuestions: Set<String> = []
    /// The pane's frame: the question card may take the height left after the conversation's minimum.
    @State private var paneFrame = CGRect.zero

    /// The label shows on split tabs, and on any pane while it is being renamed.
    private var labelShown: Bool { showLabel || model.renamingPane?.id == pane.id }

    var body: some View {
        VStack(spacing: 0) {
            if let agent = pane.agent {
                // Label and composer are system bars over the conversation: text scrolls under them and fades at the
                // scroll edges, instead of being cut by a hard rectangle.
                ConversationView(items: model.store.conversation(for: agent.sessionId), agentName: agent.name,
                                 liveCard: model.liveCardInConversation(pane),
                                 onAllow: { approve(true, $0) },
                                 onDeny: { approve(false, $0) },
                                 onOpenTerminal: { model.showTerminal = true },
                                 agentStatus: agent.status,
                                 onFresh: { if focused { focusRequest += 1 } },
                                 loadState: model.store.conversationState(for: agent.sessionId),
                                 team: agent.team,
                                 hostSetup: SessionResolver.hostSetup(kind: agent.kind, sessionId: agent.sessionId, remote: pane.composerRemote(model.store.session.remote)),
                                 scrollToken: model.sendTokens[pane.id] ?? 0,
                                 session: agent.sessionId,
                                 onLoadEarlier: { await model.store.loadEarlier(paneId: pane.id) })
                    .environment(\.logURL, model.store.logURL(for: agent.sessionId))
                    .paneMenu(pane, tab: tab)
                    .id(pane.id)   // a different pane is a different conversation: fresh scroll state, rests at the bottom
                    .paneBar(.top) {
                        VStack(spacing: 0) {
                            if labelShown { PaneLabel(pane: pane, focused: focused, zoomed: zoomed, tab: tab) }
                            TeamLine(team: agent.team)
                        }
                    }
                    .paneBar(.bottom) {
                        VStack(spacing: 0) {
                            if agent.status == .working, let q = model.queued[pane.id] {
                                QueuedStrip(queue: q, canPull: model.writesEnabled, pulling: model.pulling.contains(pane.id),
                                            onPull: { model.pullQueued(pane) })
                            }
                            if let line = model.statusLines[pane.id] { StatusLineBar(record: line) }
                            composer(agent)
                        }
                    }
                    .softScrollEdges([.top, .bottom])
            } else {
                if labelShown { PaneLabel(pane: pane, focused: focused, zoomed: zoomed, tab: tab) }
                if flat { ShellCard(pane: pane) } else {
                    ObservedTerminal(client: model.store.client, hc: pane.backend == .hc ? model.store.hcClient : nil, paneId: pane.id, terminalId: pane.terminalId,
                                     // hc: watches (never resizes the user's terminal) until the user types here.
                                     writable: TerminalPolicy.writable(backend: pane.backend, writesEnabled: model.writesEnabled,
                                                                       underSheet: model.showTerminal), focused: focused,
                                     rightClickToPane: model.rightClickToPane.contains(pane.id))
                        .paneMenu(pane, tab: tab, suppressWhenRouted: true)
                        .id(TerminalPolicy.viewKey(terminalId: pane.terminalId, backend: pane.backend, underSheet: model.showTerminal))
                        .padding(.horizontal, 14).padding(.bottom, 8)   // same inset as the pane label; the text never touches the edge
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { paneFrame = $0; QuestionProbe.paneFrame = $0 }
        .task(id: pane.cwd) { branch = await GitInfo.branch(cwd: pane.cwd, host: pane.composerRemote(model.store.session.remote)) }
        .task(id: pane.agent?.status) { await readScreenQuestion() }
        .task(id: "\(pane.agent?.status.rawValue ?? "")-\(focused)") {
            // While blocked or a tool call is pending, re-read the screen every 2 s.
            repeat {
                await model.checkPrompt(for: pane)
                guard focused, pane.agent?.status == .blocked || model.hasPending(pane) else { break }
                try? await Task.sleep(for: .seconds(2))
            } while !Task.isCancelled
        }
        .task(id: pane.agent?.sessionId) {
            // Claude Code's status line (Settings > Agents), every 2 s while shown (read-only: the wrapper's files).
            guard pane.agent?.kind == "claude" else { model.checkStatusLine(for: pane); return }
            repeat {
                model.checkStatusLine(for: pane)
                try? await Task.sleep(for: .seconds(2))
            } while !Task.isCancelled
        }
        .task(id: pane.agent?.status) {
            // While the agent works, read its own queue every 2 s (read-only).
            repeat {
                await model.checkQueue(for: pane)
                guard pane.agent?.status == .working else { break }
                try? await Task.sleep(for: .seconds(2))
            } while !Task.isCancelled
        }
    }
}

extension PaneView {
    /// One composer look for every pane. An unfocused pane shows it with a "Click to reply" placeholder; the first click
    /// focuses the pane and puts the cursor in the field.
    @ViewBuilder func composer(_ agent: Agent) -> some View {
        if let q = question(agent) {
            QuestionCard(model: q, agentName: agent.name, canSend: Self.canAnswer(q, writes: model.writesEnabled),
                         maxHeight: Self.cardHeight(pane: paneFrame.height, label: labelShown),
                         onSend: { try await answer(q, $0) },
                         onLoadAll: { try await loadAllQuestions() },
                         // The sheet shows the tab's focused pane: this one.
                         onOpenTerminal: { if let tab { model.focus(pane, in: tab) }; model.showTerminal = true })
                .id(q.toolUseId)   // fresh answers per question
                .inputColumn()
        } else if let r = ApprovalProbe.injected.flatMap({ agent.status == .blocked ? $0 : nil }) ?? model.cardInComposer(pane) {
            // A prompt the log does not have (no transcript, a log that lags, a box the log's call does not match): the
            // card from the screen, where the question card goes.
            ApprovalCard(request: r, agentName: agent.name, canRespond: true,
                         onAllow: { approve(true, r) },
                         onDeny: { approve(false, r) },
                         onOpen: { model.showTerminal = true },
                         maxHeight: Self.cardHeight(pane: paneFrame.height, label: labelShown))
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { ApprovalProbe.cardFrame = $0 }
                .id(r.toolUseId)
                .inputColumn()
        } else {
            composerField(agent)
        }
    }

    /// The conversation keeps this much height above the question card.
    static let conversationMin: CGFloat = 160

    /// The tallest the question card may be in a pane this tall (the card scrolls its options inside). Pane height 0 (not
    /// measured yet): a small card, so the first layout never pushes the pane past the window.
    static func cardHeight(pane: CGFloat, label: Bool) -> CGFloat {
        guard pane > 0 else { return 240 }
        return max(200, pane - conversationMin - (label ? 28 : 0) - 18)   // 18: the card's padding in the bar
    }

    /// The question the agent waits on: the log's last item is a pending AskUserQuestion and the agent is not working.
    /// (A killed session can leave a question pending forever; a later item or a working agent means it is gone.)
    static func liveQuestion(_ items: [ConversationItem], status: AgentStatus) -> QuestionRequest? {
        guard status != .working, let last = items.last, case .question(let q) = last.kind, q.state == .pending else { return nil }
        return q
    }

    /// The question to answer here: Claude's picker on screen, else a pending question in the log (Codex).
    private func question(_ agent: Agent) -> QuestionRequest? {
        if let q = QuestionProbe.injected, agent.status == .blocked { return q }   // tests only (nil in the app)
        if let q = screenQuestion, agent.status == .blocked, !answeredQuestions.contains(q.toolUseId) { return q }
        return Self.liveQuestion(model.store.conversation(for: agent.sessionId), status: agent.status)
            .flatMap { answeredQuestions.contains($0.toolUseId) ? nil : $0 }
    }

    /// Desk sends the answer only as picker keys (verified end to end in a named test session); a Codex async question (answered by a
    /// message) is not verified, so the card asks to answer in the terminal.
    static func canAnswer(_ q: QuestionRequest, writes: Bool) -> Bool { writes && q.delivery == .picker }

    /// READ-ONLY screen read, once each time the agent turns blocked (and after a failed answer). Sends nothing.
    private func readScreenQuestion() async {
        guard PermissionGate.readQuestion(pane.agent), let screen = model.screen(for: pane) else { screenQuestion = nil; return }
        let fresh = try? await screen.pendingQuestion(paneId: pane.id)
        // Keep the fully read request for the same picker: a read-only read has only the question on screen.
        if let fresh, let old = screenQuestion, old.toolUseId == fresh.toolUseId, old.questions.count > fresh.questions.count { return }
        screenQuestion = fresh
    }

    /// User action only (Next past the questions read so far): moves Claude's picker to read the rest, and back.
    private func loadAllQuestions() async throws -> QuestionRequest? {
        guard model.writesEnabled else { return nil }
        // Every key gated by `KeyGate.onMac` (as `loadAllQuestionsGated`): another agent or another question there now
        // stops the walk. Through the model, so the walk holds the pane's lock like the phone's.
        let model = model
        let all = try await model.loadAllQuestions(pane: pane.id, gate: KeyGate.onMac(pane.agent, allowed: { model.writesEnabled }),
                                                   expect: screenQuestion)
        if let all { screenQuestion = all }
        return all
    }

    /// User action only (Send): types the answers into the picker. On success the card shows "Sent" for a moment, then the
    /// composer comes back; on failure the screen is read again (read-only) and the card shows the error.
    /// Under the Settings page nothing is sent, whatever reaches the card (its rows may keep older closures).
    private func approve(_ allow: Bool, _ bound: PermissionRequest?) {
        guard !model.coveredBySettings else { return }
        model.approve(pane: pane, allow: allow, bound: bound)
    }

    private func answer(_ q: QuestionRequest, _ answers: [QuestionAnswer]) async throws {
        guard Self.canAnswer(q, writes: model.writesEnabled) else { return }
        guard !model.coveredBySettings else { throw QuestionCard.HeldUnderSettings() }   // not "Sent": nothing went
        let request = screenQuestion?.toolUseId == q.toolUseId ? screenQuestion ?? q : q   // the fully read one
        do {
            // Every key gated by the Mac's rule (the agent when it has an identity, else the screen before each key).
            let model = model
            try await model.answerQuestion(pane: pane.id, request: request, answers: answers,
                                           gate: KeyGate.onMac(pane.agent, allowed: { model.writesEnabled }))
            model.noteSent(pane.id)
            Task { try? await Task.sleep(for: .seconds(1.5)); answeredQuestions.insert(q.toolUseId) }
        } catch {
            await readScreenQuestion()
            throw error
        }
    }

    @ViewBuilder private func composerField(_ agent: Agent) -> some View {
        ComposerView(agentName: agent.name, readOnly: !model.writesEnabled, branch: branch,
                     draft: model.drafts[pane.id] ?? ComposerDraft(),
                     onSend: { model.send($0, to: pane) },
                     onDraft: { model.setDraft($0, for: pane.id) },
                     deliverResult: { await model.deliverResult($0, pane: pane.id, kind: agent.kind) },
                     placeholder: focused ? nil : "Click to reply to \(agent.name)",
                     focusRequest: focusRequest, remote: pane.composerRemote(model.store.session.remote),
                     commands: model.slashCommands(pane: pane, kind: agent.kind))
            .id("\(pane.id)-\(model.composerResets[pane.id] ?? 0)")   // a pulled queue starts the composer again from its draft
            .overlay {
                if !focused {
                    Button { onFocus(); focusRequest += 1 } label: { Color.clear.contentShape(Rectangle()) }
                        .buttonStyle(.plain).accessibilityLabel("Reply to \(agent.name)")
                }
            }
            .inputColumn()
    }
}

struct PaneLabel: View {
    @Environment(AppModel.self) private var model
    let pane: Pane
    let focused: Bool
    var zoomed = false
    var tab: Tab? = nil
    @State private var hoverZoom = false

    /// Focus shows in the text (primary vs secondary), not as a coloured rule.
    var body: some View {
        HStack(spacing: 6) {
            if let a = pane.agent {
                StatusGlyph(status: a.status, size: 12).opacity(focused ? 1 : 0.6)
                name(pane.label ?? a.name)
                Text(a.status.label).font(.caption).foregroundStyle(.secondary)
            } else {
                Image(systemName: "apple.terminal").imageScale(.small).foregroundStyle(focused ? .primary : .secondary)
                name(pane.label ?? "Shell")
                Text("zsh · \(pane.cwd)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if let tab, tab.isSplit {
                Button { model.toggleZoom(pane, in: tab) } label: {
                    Image(systemName: zoomed ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right")
                        .imageScale(.small).fontWeight(.semibold).foregroundStyle(.secondary)
                        .frame(width: 22, height: 20)
                        .background(Color.primary.opacity(hoverZoom ? 0.1 : 0), in: RoundedRectangle(cornerRadius: Radius.control))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain).onHover { hoverZoom = $0 }
                .help(zoomed ? "Restore panes (⇧⌘↩)" : "Zoom pane (⇧⌘↩)")
            }
        }
        .padding(.horizontal, 14).frame(height: 28)
        .contentShape(Rectangle())
        .paneMenu(pane, tab: tab)
        .onTapGesture(count: 2) { if let tab { model.toggleZoom(pane, in: tab) } }
    }

    /// The pane name, or the inline rename field. Return renames, Esc cancels, leaving the field renames.
    /// Empty clears the name.
    @ViewBuilder private func name(_ shown: String) -> some View {
        if model.renamingPane?.id == pane.id {
            InlineRenameField(initial: model.paneRenameDraft, placeholder: "Pane name",
                              onCommit: { commit($0) }, onCancel: { model.renamingPane = nil })
                .fixedSize()
        } else {
            Text(shown).font(.caption.weight(.semibold)).foregroundStyle(focused ? .primary : .secondary)
        }
    }

    private func commit(_ text: String) {
        guard model.renamingPane?.id == pane.id else { return }
        model.renamingPane = nil
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if t != (pane.label ?? "") { model.renamePane(pane, to: t) }
    }
}

/// A plain inline name field. AppKit, not a SwiftUI TextField with FocusState: when the field appears together with
/// its bar, SwiftUI moves focus to it twice, and the second move ends the edit session (focus reads false, so a
/// "leave = rename" rule closed the field at once). Here the field takes focus once when it is in a window, and the
/// end of editing says why it ended: Return or leaving the field renames, Esc cancels.
struct InlineRenameField: NSViewRepresentable {
    let initial: String
    var placeholder = ""
    let onCommit: (String) -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> Field {
        let f = Field(string: initial)
        f.placeholderString = placeholder
        f.isBordered = false; f.drawsBackground = false; f.focusRingType = .none
        f.font = .systemFont(ofSize: NSFont.preferredFont(forTextStyle: .caption1).pointSize, weight: .semibold)
        f.lineBreakMode = .byClipping; f.cell?.isScrollable = true
        f.delegate = context.coordinator
        f.setAccessibilityLabel(placeholder)
        return f
    }
    func updateNSView(_ f: Field, context: Context) {
        context.coordinator.parent = self
        SettingsFocus.cover(f, context.environment.underSettings)
    }

    final class Field: NSTextField {
        private var focusedOnce = false
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard window != nil, !focusedOnce else { return }
            focusedOnce = true
            DispatchQueue.main.async { [weak self] in
                guard let self, let w = self.window else { return }
                w.makeFirstResponder(self); self.currentEditor()?.selectAll(nil)
            }
        }
        /// As wide as the text (at least 60 pt), so the status after it does not jump far right.
        override var intrinsicContentSize: NSSize {
            let s = (stringValue.isEmpty ? (placeholderString ?? "") : stringValue) as NSString
            let w = s.size(withAttributes: [.font: font as Any]).width
            return NSSize(width: max(60, ceil(w) + 8), height: super.intrinsicContentSize.height)
        }
    }

    @MainActor final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: InlineRenameField?
        private var done = false
        private func finish(_ commit: Bool, _ text: String) {
            guard !done else { return }
            done = true
            if commit { parent?.onCommit(text) } else { parent?.onCancel() }
        }
        func controlTextDidChange(_ n: Notification) { (n.object as? NSTextField)?.invalidateIntrinsicContentSize() }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
            if sel == #selector(NSResponder.cancelOperation(_:)) { finish(false, ""); return true }
            if sel == #selector(NSResponder.insertNewline(_:)) { finish(true, control.stringValue); return true }
            return false
        }
        func controlTextDidEndEditing(_ n: Notification) { finish(true, (n.object as? NSTextField)?.stringValue ?? "") }
    }
}

struct ShellCard: View {
    let pane: Pane
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("$ cd \(pane.cwd)").font(.system(.callout, design: .monospaced))
            Spacer(minLength: 0)
        }
        .padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

/// Search: the system search field in the toolbar (⌘K opens this screen and focuses it), results in a selectable list.
/// Return opens the selected row (the first hit is selected while typing); arrows move the selection in the list;
/// double click opens.
struct SearchView: View {
    @Environment(AppModel.self) private var model
    @State private var selection: String?
    @State private var fieldShown = false
    @State private var area = CGRect.zero

    struct Hit: Identifiable { let ws: Workspace; let tab: Tab; var id: String { tab.id } }

    static func hits(_ q: String, in workspaces: [Workspace]) -> [Hit] {
        let q = q.trimmingCharacters(in: .whitespaces).lowercased()
        return workspaces.flatMap { ws in
            ws.tabs.filter { q.isEmpty || $0.label.lowercased().contains(q) || ws.label.lowercased().contains(q)
                || $0.panes.contains { $0.agent?.name.lowercased().contains(q) ?? false } }.map { Hit(ws: ws, tab: $0) }
        }
    }

    var body: some View {
        @Bindable var m = model
        let hits = Self.hits(model.searchText, in: model.store.workspaces)
        List(hits, selection: $selection) { h in
            HStack(spacing: 8) {
                StatusGlyph(status: h.tab.status)
                // Explicit styles: the list's default dims the text in a window that is not key (as the sidebar).
                Text(h.tab.label).foregroundStyle(.primary)
                if let a = h.tab.primaryAgent { Text(a.name).foregroundStyle(.secondary) }
                Spacer()
                Text(h.ws.label).foregroundStyle(.secondary)
            }
            .background { CoveredList() }   // under the Settings page the list does not take the keys
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { if h.id == hits.first?.id { ScreenColumn.probe?("search", $0, area) } }
            .listRowInsets(EdgeInsets(top: 0, leading: ScreenColumn.margin(area.width) + 8, bottom: 0, trailing: ScreenColumn.margin(area.width) + 8))
            .tag(h.id)
        }
        .listStyle(.sidebar)                // the sidebar's rounded, inset selection
        .scrollContentBackground(.hidden)   // on the window surface: no lighter list panel next to the sidebar
        .contextMenu(forSelectionType: String.self) { _ in } primaryAction: { ids in open(ids.first, in: hits) }
        .onKeyPress(.return) { open(selection, in: hits); return .handled }
        .overlay {
            if hits.isEmpty { ContentUnavailableView.search(text: model.searchText) }
        }
        // Rows in the centred screen column (row insets); the list itself spans the detail width, so the side margins scroll
        // it too and the scroll indicator stays at the window edge.
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { area = $0 }
        .searchable(text: $m.searchText, isPresented: $fieldShown, placement: .toolbar, prompt: "Workspaces, tabs, agents")
        .onSubmit(of: .search) { open(selection, in: hits) }
        .onChange(of: model.searchText, initial: true) { selection = Self.hits(model.searchText, in: model.store.workspaces).first?.id }
        .task { fieldShown = true }   // a change, once the toolbar field exists: focuses it
    }

    private func open(_ id: String?, in hits: [Hit]) {
        guard let h = hits.first(where: { $0.id == id }) ?? hits.first else { return }
        model.select(tab: h.tab, in: h.ws)
    }
}

/// The app's notice: a capsule over the panes, label colour on material (readable in light and dark).
struct NoticeCapsule: View {
    let text: String
    var body: some View {
        Label(text, systemImage: "exclamationmark.triangle.fill")
            .symbolRenderingMode(.multicolor)
            .font(.callout).padding(.horizontal, 14).padding(.vertical, 8)
            .background(.regularMaterial, in: Capsule())
            .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
    }
}
