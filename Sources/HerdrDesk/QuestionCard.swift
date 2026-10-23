import AppKit
import HerdrKit
import SwiftUI

/// Answer text as Claude Code reports it keeps the raw label ("Local file (Recommended)"); the card shows labels without it.
enum QuestionText {
    static let suffixes = [" (Recommended)", "(Recommended)", "（建議）", "（推薦）"]
    static func shown(_ answer: String) -> String {
        suffixes.reduce(answer) { $0.replacingOccurrences(of: $1, with: "") }
    }
}

/// The card's answers in progress: pure state, so keys and clicks share one path and a test can drive it.
struct QuestionFlow: Equatable {
    private(set) var questions: [QuestionRequest.Question]
    /// Every question's header from Claude's picker tab bar (empty for log items). More headers than questions: the rest
    /// are not read yet (`HerdrClient.loadAllQuestions` reads them, on a user action only).
    var headers: [String] = []
    var step = 0
    var picks: [Set<Int>]
    var other: [String]
    /// The option picked last (the card scrolls it into view).
    var lastPick: Int?

    init(_ questions: [QuestionRequest.Question], headers: [String] = []) {
        self.questions = questions
        self.headers = headers
        picks = questions.map { _ in [] }
        other = questions.map { _ in "" }
    }

    var question: QuestionRequest.Question { questions[step] }
    /// Questions in all, read or not.
    var total: Int { max(questions.count, headers.count) }
    var isLast: Bool { step == total - 1 }
    /// Going on from here needs the questions not read yet.
    var needsLoad: Bool { step == questions.count - 1 && !isLast }
    /// The header chip: the question's own, else the tab bar's (Claude's screen read leaves it empty with several).
    var header: String { question.header.isEmpty && headers.indices.contains(step) ? headers[step] : question.header }

    /// All questions, once read: answers so far stay.
    mutating func load(_ all: [QuestionRequest.Question]) {
        guard all.count >= questions.count else { return }
        questions = all
        picks += Array(repeating: [], count: all.count - picks.count)
        other += Array(repeating: "", count: all.count - other.count)
    }
    /// The current question has a pick or an "Other" answer.
    var valid: Bool { !picks[step].isEmpty || !other[step].trimmingCharacters(in: .whitespaces).isEmpty }
    /// Number key of the "Other" field (single choice): the one after the last option.
    var otherKey: Int { question.options.count + 1 }

    enum Digit { case picked, other, ignored }
    /// 1…n picks option n (toggles in multiSelect); n+1 goes to "Other" in single choice.
    mutating func digit(_ n: Int) -> Digit {
        if (1...question.options.count).contains(n) { pick(n - 1); return .picked }
        if n == otherKey, !question.multiSelect { return .other }
        return .ignored
    }

    mutating func pick(_ i: Int) {
        lastPick = i
        if question.multiSelect {
            if picks[step].contains(i) { picks[step].remove(i) } else { picks[step].insert(i) }
        } else {
            picks[step] = [i]; other[step] = ""
        }
    }

    /// Typing an "Other" answer in single choice replaces the pick.
    mutating func setOther(_ s: String) {
        other[step] = s
        if !question.multiSelect, !s.trimmingCharacters(in: .whitespaces).isEmpty { picks[step] = [] }
    }

    /// Return: next question, or true when the last one is answered (time to send).
    mutating func advance() -> Bool {
        guard valid else { return false }
        if isLast { return true }
        guard step + 1 < questions.count else { return false }   // not read yet: `load` first
        step += 1
        return false
    }

    mutating func back() { if step > 0 { step -= 1 } }

    /// What `HerdrClient.answerQuestion` takes: 0-based picks in option order, and the "Other" text if any.
    func values() -> [QuestionAnswer] {
        questions.indices.map { k in
            let o = other[k].trimmingCharacters(in: .whitespaces)
            return QuestionAnswer(picks: picks[k].sorted(), other: o.isEmpty ? nil : o)
        }
    }

    /// One string per question, as HerdrKit reports answers: labels in option order joined ", ", then the "Other" text.
    func answers() -> [String] {
        questions.indices.map { k in
            var parts = picks[k].sorted().map { questions[k].options[$0].label }
            let o = other[k].trimmingCharacters(in: .whitespaces)
            if !o.isEmpty { parts.append(o) }
            return parts.joined(separator: ", ")
        }
    }
}

/// Test hook for the live check (`--test-question-live`): what the card did. nil in the app.
@MainActor enum QuestionProbe {
    enum Event { case appeared(QuestionRequest), flow(QuestionFlow), loaded(Int), sent, failed(String) }
    static var report: ((Event) -> Void)?
    /// Tests: the question every blocked pane shows, as if read from its screen.
    static var injected: QuestionRequest?
    /// The card's global frame, last reported (tests).
    static var cardFrame = CGRect.zero
    /// The pane's global frame, last reported (tests).
    static var paneFrame = CGRect.zero
}

/// AskUserQuestion in the composer slot: number keys pick, Return goes on or sends, Esc leaves "Other" or goes back one
/// question. `canSend` off (read-only Desk, or an answer Desk cannot deliver): the card says to answer in the terminal.
struct QuestionCard: View {
    @Environment(\.underSettings) private var underSettings
    let model: QuestionRequest
    let agentName: String
    var canSend = false
    /// The card's height limit; the question and its options scroll inside when they do not fit.
    var maxHeight: CGFloat = .infinity
    /// Delivers the answers (HerdrClient.answerQuestion); a thrown error shows on the card and the answers stay.
    var onSend: ([QuestionAnswer]) async throws -> Void = { _ in }
    /// Reads the questions not read yet (HerdrClient.loadAllQuestions: moves Claude's picker). Called only when the user
    /// goes on from the last read question, never on its own.
    var onLoadAll: () async throws -> QuestionRequest? = { nil }
    /// Tests: the answers in progress after each change.
    var onFlow: ((QuestionFlow) -> Void)? = nil
    @State private var flow: QuestionFlow
    @State private var sent = false
    @State private var busy = false
    @State private var problem: String?
    /// Height of the question and options (the scrolled part) and of the rest of the card.
    @State private var bodyHeight: CGFloat = 0
    @State private var chrome: CGFloat = 0
    @FocusState private var focus: Field?
    enum Field: Hashable { case card, other }
    @State private var anchor = ViewAnchor()

    init(model: QuestionRequest, agentName: String, canSend: Bool = false, maxHeight: CGFloat = .infinity,
         onSend: @escaping ([QuestionAnswer]) async throws -> Void = { _ in },
         onLoadAll: @escaping () async throws -> QuestionRequest? = { nil },
         onFlow: ((QuestionFlow) -> Void)? = nil) {
        self.model = model; self.agentName = agentName; self.canSend = canSend; self.maxHeight = maxHeight; self.onSend = onSend; self.onLoadAll = onLoadAll
        self.onFlow = onFlow
        _flow = State(initialValue: QuestionFlow(model.questions, headers: model.headers))
    }

    /// Words for a failed send or read. Nothing was typed for `.notOnScreen` and `.unsupported`.
    /// The answer was held back: the Settings page covered the card when it was to be sent.
    struct HeldUnderSettings: Error {}

    static func problemText(_ error: Error) -> String {
        if error is HeldUnderSettings { return "Not sent while Settings was open. Press Return to send." }
        return switch error as? QuestionAdapter.Failure {
        case .notOnScreen?: "The question is no longer on screen. Nothing was sent."
        case .unsupported(let why)?: "Desk cannot send this answer (\(why)). Nothing was sent. Answer in the terminal."
        case .stuck?: "The terminal did not move on as expected. Check it before you try again."
        case .stillOnScreen?: "The question is still on screen. Check the terminal."
        case nil: "Could not reach the terminal: \(error.localizedDescription)"
        }
    }

    var body: some View {
        Group {
            if sent {
                Text("Sent. \(agentName) is continuing.").font(.callout).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity).padding(.vertical, 10)
            } else {
                card
            }
        }
        .onAppear { onFlow?(flow); QuestionProbe.report?(.appeared(model)); QuestionProbe.report?(.flow(flow)) }
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { QuestionProbe.cardFrame = $0 }
        .onChange(of: flow) { _, f in onFlow?(f); QuestionProbe.report?(.flow(f)) }
    }

    private var card: some View {
        let q = flow.question
        return VStack(alignment: .leading, spacing: 16) {
            head(q)
            // The question and options scroll inside a capped height, so a long question never makes the card (and the
            // pane) taller than the window. Its height is measured, never taken from the text's own (narrow-width) size.
            ScrollViewReader { reader in
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(q.text).font(.title3.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
                            if q.multiSelect { Text("Choose any.").font(.callout).foregroundStyle(.secondary) }
                        }
                        optionGroup(q)
                    }
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { bodyHeight = $0 }
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(height: scrollHeight)
                .onChange(of: flow.lastPick) { _, i in if let i { withAnimation { reader.scrollTo(i) } } }
                .onChange(of: focus) { _, f in if f == .other { withAnimation { reader.scrollTo(Self.otherRowID) } } }
            }
            foot(q)
        }
        .padding(16)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { chrome = $0 - scrollHeight }
        .glassCard(radius: Radius.bubble)
        .overlay(RoundedRectangle(cornerRadius: Radius.bubble, style: .continuous).strokeBorder(Color.primary.opacity(0.1), lineWidth: 0.5))
        .shadow(color: .black.opacity(0.1), radius: 12, y: 4)
        .focusable()
        .focusEffectDisabled()
        .focused($focus, equals: .card)
        .onKeyPress(phases: .down) { press in key(press) }
        .onAppear { if !underSettings { focus = .card } }
        .background(ViewAnchor.Probe(anchor: anchor))
        .onChange(of: underSettings) { _, on in if !on { focusAfterBack() } }   // came under the page: Back focuses it
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(agentName) asks you")
    }

    /// The scrolled part's height: all of it when it fits under `maxHeight`, else what is left after the rest of the card.
    private var scrollHeight: CGFloat { min(bodyHeight, max(80, maxHeight - chrome)) }

    private static let otherRowID = "other"

    private func head(_ q: QuestionRequest.Question) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "questionmark.bubble.fill").font(.system(size: 20))
                .symbolRenderingMode(.hierarchical).foregroundStyle(Color.orange)
                .accessibilityHidden(true)
            Text("\(Text(agentName).font(.headline).foregroundStyle(.primary)) asks you").foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 8)
            if !flow.header.isEmpty {
                Text(flow.header).font(.caption.weight(.medium)).foregroundStyle(.secondary).lineLimit(1)
                    .padding(.horizontal, 8).padding(.vertical, 2)
                    .background(.quaternary, in: Capsule())
            }
            if flow.total > 1 {
                Text("\(flow.step + 1) of \(flow.total)").font(.callout).monospacedDigit().foregroundStyle(.secondary)
                    .accessibilityLabel("Question \(flow.step + 1) of \(flow.total)")
            }
        }
    }

    /// Options and "Other" as one inset group (System Settings style): rows with hairline separators.
    private func optionGroup(_ q: QuestionRequest.Question) -> some View {
        VStack(spacing: 0) {
            ForEach(Array(q.options.enumerated()), id: \.offset) { i, o in
                QuestionOptionRow(index: i, option: o, multi: q.multiSelect, on: flow.picks[flow.step].contains(i)) {
                    withAnimation(.spring) { flow.pick(i) }
                    focus = .card
                }
                .id(i)
                Divider().padding(.leading, 44)
            }
            otherRow(q).id(Self.otherRowID)
        }
        .background(Color.primary.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius: Radius.card, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Radius.card, style: .continuous).strokeBorder(Color.primary.opacity(0.09), lineWidth: 0.5))
    }

    private func otherRow(_ q: QuestionRequest.Question) -> some View {
        let on = !flow.other[flow.step].trimmingCharacters(in: .whitespaces).isEmpty
        return HStack(spacing: 12) {
            Image(systemName: q.multiSelect ? (on ? "checkmark.square.fill" : "square") : (on ? "checkmark.circle.fill" : "circle"))
                .font(.system(size: 17)).foregroundStyle(on ? Color.accentColor : Color.secondary.opacity(0.6))
                .frame(width: 20).accessibilityHidden(true)
            OtherField(text: Binding(get: { flow.other[flow.step] }, set: { flow.setOther($0) }),
                       placeholder: "Other: type your own answer",
                       onSubmit: { returnKey() }, onEscape: { focus = .card })
                .focused($focus, equals: .other)
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: Radius.control, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: Radius.control, style: .continuous)
                    .strokeBorder(focus == .other ? Color.accentColor.opacity(0.8) : Color.primary.opacity(0.12), lineWidth: focus == .other ? 1.5 : 0.5))
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .background(on ? Color.accentColor.opacity(0.12) : Color.clear)
        .id(flow.step)   // a fresh field per question
    }

    private func foot(_ q: QuestionRequest.Question) -> some View {
        HStack(spacing: 8) {
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle.fill").symbolRenderingMode(.multicolor)
                    .font(.caption).foregroundStyle(.secondary).lineLimit(2)
            } else if !canSend {
                HStack(spacing: 4) {
                    Text("Answer in the terminal")
                    KbdGroup(caps: ["⌃", "`"])
                    Text(": Desk cannot send this answer.").lineLimit(2)
                }
                .font(.caption).foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
            } else {
                hint(q)
            }
            Spacer(minLength: 8)
            if busy { ProgressView().controlSize(.small) }
            if flow.step > 0 { Button("Back") { flow.back(); focus = .card } }
            Button(flow.isLast ? (flow.total > 1 ? "Send Answers" : "Send") : "Next") { returnKey() }
                .buttonStyle(.borderedProminent)
                .disabled(busy || !flow.valid || (flow.isLast && !canSend))
        }
        .controlSize(.regular)
    }

    /// Key hints as keycaps: "1 – 3 choose · ↩ next".
    private func hint(_ q: QuestionRequest.Question) -> some View {
        HStack(spacing: 6) {
            KbdGroup(caps: q.options.count > 1 ? ["1", "–", "\(q.options.count)"] : ["1"])
            Text(q.multiSelect ? "toggle" : "choose")
            KbdGroup(caps: ["↩"])
            Text(flow.isLast ? "send" : "next")
        }
        .font(.caption).foregroundStyle(.tertiary).lineLimit(1)
        .accessibilityElement(children: .combine)
    }

    private func returnKey() {
        guard !underSettings else { return }
        focus = .card
        guard !busy else { return }
        if flow.needsLoad, flow.valid {
            // The user goes on past the questions read so far: read the rest now (this moves the picker and back).
            busy = true; problem = nil
            Task {
                defer { busy = false }
                do {
                    if let all = try await onLoadAll(), all.questions.count > flow.questions.count {
                        QuestionProbe.report?(.loaded(all.questions.count))
                        flow.load(all.questions); _ = flow.advance()
                    } else { problem = "Could not read the next question. Answer in the terminal."; QuestionProbe.report?(.failed(problem ?? "")) }
                } catch { problem = Self.problemText(error); QuestionProbe.report?(.failed(problem ?? "")) }
            }
            return
        }
        guard flow.advance(), canSend else { return }
        busy = true; problem = nil
        Task {
            defer { busy = false }
            do { try await onSend(flow.values()); sent = true; QuestionProbe.report?(.sent) }
            catch { problem = Self.problemText(error); QuestionProbe.report?(.failed(problem ?? "")) }
        }
    }

    /// After this update, when the card is enabled again. On macOS 15 a focus set inside a window's content does not make
    /// that content the first responder (macOS 26 does): the card came while the page held the keys, the window itself
    /// is the first responder after Back, and the keys never reach the card. So the content takes it first, then the card.
    private func focusAfterBack() {
        DispatchQueue.main.async {
            if let v = anchor.view, let win = v.window, let content = win.contentView, v.isDescendant(of: content),
               win.firstResponder == nil || win.firstResponder === win { win.makeFirstResponder(content) }
            DispatchQueue.main.async { focus = .card }
        }
    }

    private func key(_ press: KeyPress) -> KeyPress.Result {
        guard !underSettings else { return .ignored }
        guard press.modifiers.subtracting(.shift).isEmpty else { return .ignored }
        switch press.key {
        case .return: returnKey(); return .handled
        case .escape:
            if flow.step > 0 { flow.back(); return .handled }
            return .ignored
        default:
            guard let n = Int(press.characters), (1...9).contains(n) else { return .ignored }
            var next = flow
            let result = next.digit(n)
            if result == .picked { withAnimation(.spring) { flow = next } }
            switch result {
            case .picked: return .handled
            case .other: focus = .other; return .handled
            case .ignored: return .ignored
            }
        }
    }
}

/// The AppKit view behind a SwiftUI view, for its window.
private final class ViewAnchor {
    weak var view: NSView?
    struct Probe: NSViewRepresentable {
        let anchor: ViewAnchor
        func makeNSView(context: Context) -> NSView { let v = NSView(); anchor.view = v; return v }
        func updateNSView(_ view: NSView, context: Context) {}
    }
}

/// One option in the question card's group: a radio (single choice) or checkbox (multiSelect), title and detail. The
/// number key shows as a faint keycap on hover only; the footer lists the keys.
private struct QuestionOptionRow: View {
    let index: Int
    let option: QuestionRequest.Option
    let multi: Bool
    let on: Bool
    let action: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: multi ? (on ? "checkmark.square.fill" : "square") : (on ? "checkmark.circle.fill" : "circle"))
                    .font(.system(size: 17)).foregroundStyle(on ? Color.accentColor : Color.secondary.opacity(0.6))
                    .frame(width: 20, height: 20)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Text(option.label).font(.body.weight(.medium))
                        if option.recommended {
                            Text("Recommended").font(.caption.weight(.medium)).foregroundStyle(Color.accentColor)
                                .padding(.horizontal, 6).padding(.vertical, 1)
                                .background(Color.accentColor.opacity(0.12), in: Capsule())
                        }
                    }
                    if !option.detail.isEmpty {
                        Text(option.detail).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                if hover, index < 9 { Kbd(key: "\(index + 1)").opacity(0.7) }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(on ? Color.accentColor.opacity(0.12) : hover ? Color.primary.opacity(0.05) : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityLabel(option.recommended ? "\(option.label), recommended" : option.label)
    }
}

/// "Other" text field. Return while the input method has marked text commits the text and never sends; Esc leaves.
struct OtherField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    var onSubmit: () -> Void
    var onEscape: () -> Void

    func makeNSView(context: Context) -> NSTextField {
        let f = NSTextField()
        f.isBordered = false; f.drawsBackground = false; f.focusRingType = .none
        f.font = .systemFont(ofSize: NSFont.systemFontSize)
        f.placeholderString = placeholder
        f.delegate = context.coordinator
        f.cell?.lineBreakMode = .byTruncatingTail
        f.setAccessibilityLabel("Other answer")
        return f
    }
    func updateNSView(_ f: NSTextField, context: Context) {
        context.coordinator.parent = self
        SettingsFocus.cover(f, context.environment.underSettings)
        let editing = (f.currentEditor() as? NSTextView)?.hasMarkedText() == true
        if f.stringValue != text, !editing { f.stringValue = text }
    }
    func makeCoordinator() -> Coordinator { Coordinator(self) }

    @MainActor final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: OtherField
        init(_ p: OtherField) { parent = p }
        func controlTextDidChange(_ n: Notification) {
            guard let f = n.object as? NSTextField, let tv = f.currentEditor() as? NSTextView, !tv.hasMarkedText() else { return }
            parent.text = f.stringValue
        }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
            switch sel {
            case #selector(NSResponder.insertNewline(_:)):
                if textView.hasMarkedText() { textView.unmarkText(); parent.text = textView.string; return true }
                parent.text = textView.string
                parent.onSubmit()
                return true
            case #selector(NSResponder.cancelOperation(_:)):
                parent.onEscape(); return true
            default:
                return false
            }
        }
    }
}

/// An answered question in the transcript: each question and its answer, then a check line.
struct QuestionAnsweredRow: View {
    let model: QuestionRequest
    var time: Date? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            switch model.state {
            case .answered(let answers):
                ForEach(Array(model.questions.enumerated()), id: \.offset) { k, q in
                    let a = answers.flatMap { $0.indices.contains(k) ? $0[k] : nil }
                    row(q.text, a.map { $0.isEmpty ? "Skipped" : QuestionText.shown($0) })
                }
                meta("checkmark.circle.fill", .green, "Answered")
            case .declined:
                ForEach(Array(model.questions.enumerated()), id: \.offset) { _, q in row(q.text, nil) }
                meta("xmark.circle.fill", .secondary, "Declined")
            case .pending:
                EmptyView()
            }
        }
        .font(.callout)
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: Radius.card))
        .overlay(RoundedRectangle(cornerRadius: Radius.card).strokeBorder(Color.primary.opacity(0.09), lineWidth: 0.5))
    }

    private func row(_ q: String, _ a: String?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(q).foregroundStyle(.secondary)
            if let a { Text("\(Text("→").foregroundStyle(.tertiary)) \(Text(a).fontWeight(.semibold))") }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private func meta(_ symbol: String, _ tint: Color, _ word: String) -> some View {
        HStack(spacing: 5) {
            Image(systemName: symbol).foregroundStyle(tint)
            Text(time.map { "\(word) at \($0.formatted(date: .omitted, time: .shortened))" } ?? word)
        }
        .font(.caption).foregroundStyle(.tertiary)
    }
}
