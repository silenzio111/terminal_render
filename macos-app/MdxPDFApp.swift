import AppKit
import Foundation
import SwiftUI
import UniformTypeIdentifiers

private let markdownExtensions: Set<String> = ["md", "markdown", "mdown"]

enum ExportState {
    case waiting
    case running
    case succeeded
    case failed(String)
}

enum PDFPaginationMode: String, CaseIterable, Identifiable {
    case paginated
    case continuous

    var id: String { rawValue }

    var title: String {
        switch self {
        case .paginated: return "分页"
        case .continuous: return "不分页"
        }
    }

    var detail: String {
        switch self {
        case .paginated: return "按 A4 页面导出"
        case .continuous: return "输出为连续长页"
        }
    }

    var iconName: String {
        switch self {
        case .paginated: return "doc.on.doc"
        case .continuous: return "arrow.down.to.line"
        }
    }
}

struct ExportItem: Identifiable {
    let id = UUID()
    let inputURL: URL
    var state: ExportState = .waiting

    var outputURL: URL {
        inputURL.deletingPathExtension().appendingPathExtension("pdf")
    }
}

private struct ExportJob {
    let inputURL: URL
    let outputURL: URL
    let paginate: Bool
}

private struct ExportResult {
    let inputPath: String
    let succeeded: Bool
    let message: String?
}

final class OpenFileCoordinator {
    static let shared = OpenFileCoordinator()

    private var pendingURLs: [URL] = []
    private var receiver: (([URL]) -> Void)?

    func register(receiver: @escaping ([URL]) -> Void) {
        self.receiver = receiver
        deliverPendingURLs()
    }

    func enqueue(_ urls: [URL]) {
        pendingURLs.append(contentsOf: urls)
        deliverPendingURLs()
    }

    private func deliverPendingURLs() {
        guard let receiver, !pendingURLs.isEmpty else { return }
        let urls = pendingURLs
        pendingURLs.removeAll()
        receiver(urls)
    }
}

final class AppModel: ObservableObject {
    private static let paginationModeKey = "pdfPaginationMode"

    @Published private(set) var items: [ExportItem] = []
    @Published private(set) var isExporting = false
    @Published var alertMessage: String?
    @Published var paginationMode: PDFPaginationMode {
        didSet {
            UserDefaults.standard.set(
                paginationMode.rawValue,
                forKey: Self.paginationModeKey
            )
        }
    }

    private var pendingFinderURLs: [URL] = []

    init() {
        let savedMode = UserDefaults.standard.string(forKey: Self.paginationModeKey)
        paginationMode = PDFPaginationMode(rawValue: savedMode ?? "") ?? .paginated
    }

    var canExport: Bool {
        !isExporting && items.contains(where: shouldExport)
    }

    var statusMessage: String {
        guard !items.isEmpty else {
            return "尚未添加 Markdown 文件"
        }

        let completed = items.reduce(0) { count, item in
            if case .succeeded = item.state {
                return count + 1
            }
            return count
        }
        let failed = items.reduce(0) { count, item in
            if case .failed = item.state {
                return count + 1
            }
            return count
        }

        if isExporting {
            return "正在排版 \(items.count) 个文件"
        }
        if failed > 0 {
            return "已完成 \(completed) 个，失败 \(failed) 个"
        }
        if completed == items.count {
            return "已完成 \(completed) 个 PDF"
        }
        return "已添加 \(items.count) 个 Markdown 文件"
    }

    func chooseFiles() {
        let panel = NSOpenPanel()
        panel.title = "选择 Markdown 文件"
        panel.message = "可以同时选择多个 .md 文件"
        panel.allowedContentTypes = markdownContentTypes
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false

        guard panel.runModal() == .OK else { return }
        add(urls: panel.urls)
    }

    func receiveFromFinder(_ urls: [URL]) {
        if isExporting {
            pendingFinderURLs.append(contentsOf: urls)
            return
        }

        // Each Finder invocation represents a new export batch. Do not keep
        // the previous batch around or include it in the next command.
        items.removeAll()
        alertMessage = nil
        add(urls: urls)
        startExport()
    }

    func addDroppedFiles(_ providers: [NSItemProvider]) -> Bool {
        let group = DispatchGroup()
        let lock = NSLock()
        var urls: [URL] = []

        for provider in providers {
            group.enter()
            provider.loadDataRepresentation(forTypeIdentifier: UTType.fileURL.identifier) {
                data, _ in
                defer { group.leave() }
                guard
                    let data,
                    let url = URL(dataRepresentation: data, relativeTo: nil)
                else {
                    return
                }

                lock.lock()
                urls.append(url)
                lock.unlock()
            }
        }

        group.notify(queue: .main) { [weak self] in
            self?.receiveFromFinder(urls)
        }
        return true
    }

    func startExport() {
        guard canExport else { return }

        let paginate = paginationMode == .paginated
        let jobs = items.filter(shouldExport).map {
            ExportJob(
                inputURL: $0.inputURL,
                outputURL: $0.outputURL,
                paginate: paginate
            )
        }
        let jobPaths = Set(jobs.map { $0.inputURL.path })
        isExporting = true
        for index in items.indices {
            if jobPaths.contains(items[index].inputURL.path) {
                items[index].state = .running
            }
        }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }

            for job in jobs {
                let result = self.run(job)
                DispatchQueue.main.async { [weak self] in
                    self?.apply(result)
                }
            }

            DispatchQueue.main.async { [weak self] in
                self?.isExporting = false
                self?.startPendingFinderExport()
            }
        }
    }

    func clearFiles() {
        guard !isExporting else { return }
        pendingFinderURLs.removeAll()
        items.removeAll()
        alertMessage = nil
    }

    func openOutput(for item: ExportItem) {
        guard FileManager.default.fileExists(atPath: item.outputURL.path) else {
            alertMessage = "PDF 尚未生成：\(item.outputURL.path)"
            return
        }
        NSWorkspace.shared.open(item.outputURL)
    }

    func revealOutput(for item: ExportItem) {
        let url = FileManager.default.fileExists(atPath: item.outputURL.path)
            ? item.outputURL
            : item.inputURL
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    private func add(urls: [URL]) {
        let existing = Set(items.map { $0.inputURL.standardizedFileURL.path })
        let validURLs = urls
            .map { $0.standardizedFileURL }
            .filter { url in
                markdownExtensions.contains(url.pathExtension.lowercased())
                    && FileManager.default.fileExists(atPath: url.path)
            }

        for url in validURLs where !existing.contains(url.path) {
            items.append(ExportItem(inputURL: url))
        }

        if validURLs.isEmpty, !urls.isEmpty {
            alertMessage = "只支持 .md、.markdown 和 .mdown 文件。"
        }
    }

    private func shouldExport(_ item: ExportItem) -> Bool {
        switch item.state {
        case .waiting, .failed:
            return true
        case .running, .succeeded:
            return false
        }
    }

    private func startPendingFinderExport() {
        guard !pendingFinderURLs.isEmpty else { return }
        let urls = pendingFinderURLs
        pendingFinderURLs.removeAll()
        receiveFromFinder(urls)
    }

    private func apply(_ result: ExportResult) {
        guard let index = items.firstIndex(where: {
            $0.inputURL.path == result.inputPath
        }) else {
            return
        }

        if result.succeeded {
            items[index].state = .succeeded
        } else {
            let message = result.message ?? "mdx 导出失败"
            items[index].state = .failed(message)
            alertMessage = "\(items[index].inputURL.lastPathComponent)：\(message)"
        }
    }

    private func run(_ job: ExportJob) -> ExportResult {
        guard let executableURL = Bundle.main.url(forResource: "mdx", withExtension: nil) else {
            return ExportResult(
                inputPath: job.inputURL.path,
                succeeded: false,
                message: "App Bundle 中缺少 mdx 排版引擎"
            )
        }

        let process = Process()
        let stdout = Pipe()
        let stderr = Pipe()
        process.executableURL = executableURL
        process.arguments = [
            "--pdf",
            job.paginate ? "--paginate" : "--continuous",
            job.inputURL.path
        ]
        process.currentDirectoryURL = job.inputURL.deletingLastPathComponent()
        process.environment = processEnvironment()
        process.standardOutput = stdout
        process.standardError = stderr

        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return ExportResult(
                inputPath: job.inputURL.path,
                succeeded: false,
                message: error.localizedDescription
            )
        }

        let outputData = stdout.fileHandleForReading.readDataToEndOfFile()
        let errorData = stderr.fileHandleForReading.readDataToEndOfFile()
        let details = String(data: errorData + outputData, encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\n", with: " ")

        if process.terminationStatus == 0,
           FileManager.default.fileExists(atPath: job.outputURL.path) {
            return ExportResult(
                inputPath: job.inputURL.path,
                succeeded: true,
                message: nil
            )
        }

        return ExportResult(
            inputPath: job.inputURL.path,
            succeeded: false,
            message: details?.isEmpty == false ? details : "mdx 未生成 PDF"
        )
    }

    private func processEnvironment() -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let bundledBin = Bundle.main.resourceURL?.appendingPathComponent("bin").path
        let currentPath = environment["PATH"]

        let paths = [
            bundledBin,
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "\(home)/.cargo/bin",
            "\(home)/.local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
            currentPath
        ].compactMap { $0 }

        environment["PATH"] = paths.joined(separator: ":")
        return environment
    }

    private var markdownContentTypes: [UTType] {
        markdownExtensions.compactMap { UTType(filenameExtension: $0) }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(
        _ sender: NSApplication
    ) -> Bool {
        true
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let urls = CommandLine.arguments.dropFirst()
            .map { URL(fileURLWithPath: $0) }
            .filter(Self.isMarkdownFile)

        if !urls.isEmpty {
            OpenFileCoordinator.shared.enqueue(urls)
        }
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        let markdownURLs = urls.filter(Self.isMarkdownFile)
        guard !markdownURLs.isEmpty else { return }

        NSApp.activate(ignoringOtherApps: true)
        OpenFileCoordinator.shared.enqueue(markdownURLs)
    }

    private static func isMarkdownFile(_ url: URL) -> Bool {
        markdownExtensions.contains(url.pathExtension.lowercased())
            && FileManager.default.fileExists(atPath: url.path)
    }
}

@main
struct MdxPDFApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self)
    private var appDelegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        Window("mdx PDF 排版", id: "main") {
            ContentView()
                .environmentObject(model)
        }
        .defaultSize(width: 760, height: 500)
        .windowResizability(.contentSize)
    }
}

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            header
            pdfOptions
            Divider()

            if model.items.isEmpty {
                emptyState
            } else {
                fileList
            }

            Divider()
            footer
        }
        .padding(24)
        .frame(minWidth: 720, minHeight: 440)
        .onAppear {
            OpenFileCoordinator.shared.register { urls in
                model.receiveFromFinder(urls)
            }
        }
        .onDrop(
            of: [UTType.fileURL.identifier],
            isTargeted: nil,
            perform: model.addDroppedFiles
        )
        .alert(
            "导出失败",
            isPresented: Binding(
                get: { model.alertMessage != nil },
                set: { if !$0 { model.alertMessage = nil } }
            )
        ) {
            Button("确定") { model.alertMessage = nil }
        } message: {
            Text(model.alertMessage ?? "")
        }
    }

    private var header: some View {
        HStack(alignment: .center, spacing: 16) {
            Image(systemName: "doc.richtext")
                .font(.system(size: 34, weight: .medium))
                .foregroundStyle(.tint)

            VStack(alignment: .leading, spacing: 4) {
                Text("mdx PDF 排版")
                    .font(.system(size: 24, weight: .semibold))
                Text("使用 Typst 和 Elegant Paper 生成 PDF")
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button {
                model.chooseFiles()
            } label: {
                Label("添加 Markdown", systemImage: "plus")
            }
        }
        .padding(.bottom, 20)
    }

    private var emptyState: some View {
        VStack(spacing: 14) {
            Spacer()

            Image(systemName: "arrow.down.doc")
                .font(.system(size: 42, weight: .light))
                .foregroundStyle(.secondary)
            Text("将 Markdown 文件拖到这里")
                .font(.title3)
            Text("也可以在 Finder 中右键选择“打开方式”")
                .foregroundStyle(.secondary)

            Button {
                model.chooseFiles()
            } label: {
                Label("选择文件", systemImage: "folder")
            }

            Spacer()
        }
        .frame(maxWidth: .infinity)
    }

    private var pdfOptions: some View {
        HStack(spacing: 12) {
            Label("PDF 页面", systemImage: "doc.text")
                .font(.callout.weight(.medium))

            Picker("PDF 页面", selection: $model.paginationMode) {
                ForEach(PDFPaginationMode.allCases) { mode in
                    Label(mode.title, systemImage: mode.iconName)
                        .tag(mode)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .frame(width: 220)
            .disabled(model.isExporting)
            .help("选择 PDF 是否按 A4 页面分页")

            Text(model.paginationMode.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)

            Spacer()
        }
        .padding(.bottom, 16)
    }

    private var fileList: some View {
        ScrollView {
            LazyVStack(spacing: 8) {
                ForEach(model.items) { item in
                    ExportRow(
                        item: item,
                        openOutput: { model.openOutput(for: item) },
                        revealOutput: { model.revealOutput(for: item) }
                    )
                }
            }
            .padding(.vertical, 16)
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Text(model.statusMessage)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)

            Spacer()

            Button {
                model.clearFiles()
            } label: {
                Label("清空", systemImage: "trash")
            }
            .disabled(model.items.isEmpty || model.isExporting)

            Button {
                model.startExport()
            } label: {
                Label("开始排版", systemImage: "play.fill")
            }
            .keyboardShortcut(.defaultAction)
            .disabled(!model.canExport)
        }
        .padding(.top, 18)
    }
}

private struct ExportRow: View {
    let item: ExportItem
    let openOutput: () -> Void
    let revealOutput: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: iconName)
                .font(.system(size: 20, weight: .medium))
                .foregroundStyle(iconColor)
                .frame(width: 28)

            VStack(alignment: .leading, spacing: 4) {
                Text(item.inputURL.lastPathComponent)
                    .font(.headline)
                    .lineLimit(1)
                Text(item.outputURL.lastPathComponent)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()
            stateLabel

            if case .succeeded = item.state {
                Button(action: openOutput) {
                    Image(systemName: "arrow.up.right.square")
                }
                .buttonStyle(.borderless)
                .help("打开 PDF")

                Button(action: revealOutput) {
                    Image(systemName: "folder")
                }
                .buttonStyle(.borderless)
                .help("在 Finder 中显示")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Color(nsColor: .controlBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    @ViewBuilder
    private var stateLabel: some View {
        switch item.state {
        case .waiting:
            Label("等待", systemImage: "circle")
                .foregroundStyle(.secondary)
        case .running:
            HStack(spacing: 8) {
                ProgressView()
                    .controlSize(.small)
                Text("排版中")
            }
            .foregroundStyle(.secondary)
        case .succeeded:
            Label("已完成", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .failed(let message):
            Text(message)
                .font(.caption)
                .foregroundStyle(.red)
                .lineLimit(2)
        }
    }

    private var iconName: String {
        switch item.state {
        case .waiting: return "doc.text"
        case .running: return "gearshape.2"
        case .succeeded: return "checkmark.circle"
        case .failed: return "exclamationmark.triangle"
        }
    }

    private var iconColor: Color {
        switch item.state {
        case .waiting, .running: return .secondary
        case .succeeded: return .green
        case .failed: return .red
        }
    }
}
