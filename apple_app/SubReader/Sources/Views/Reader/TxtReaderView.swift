// TxtReaderView — Reading view for plain-text files.
//
// Renders TXT content natively in Swift without the Rust engine.
// Supports chapter navigation, scroll progress tracking, and theme settings.

import SwiftUI
import ReaderModels
import ReaderBridge

struct TxtReaderView: View {
    let book: LibraryBook

    @EnvironmentObject var appState: AppState
    @EnvironmentObject var container: DIContainer

    @State private var txtBook: TxtContentStore.TxtBook?
    @State private var currentChapterIndex: Int = 0
    @State private var attributedContent: NSAttributedString = NSAttributedString()
    @State private var isLoading = true
    @State private var errorMessage: String?
    @State private var scrollPercentage: Double = 0

    @AppStorage("fontSize") private var fontSize: Double = 16
    @AppStorage("lineSpacing") private var lineSpacing: Double = 1.5
    @AppStorage("fontName") private var fontName: String = "System"

    var body: some View {
        ZStack {
            if isLoading {
                ProgressView("Loading...")
            } else if let error = errorMessage {
                VStack(spacing: 12) {
                    Image(systemName: "exclamationmark.triangle")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text(error)
                        .foregroundStyle(.secondary)
                }
            } else {
                VStack(spacing: 0) {
                    AttributedTextView(
                        attributedString: attributedContent,
                        onScroll: { pct in scrollPercentage = pct }
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                    bottomBar
                }
            }
        }
        .navigationTitle(book.metadata.title)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    navigateChapter(offset: -1)
                } label: {
                    Image(systemName: "chevron.left")
                }
                .disabled(currentChapterIndex <= 0)

                Text(chapterIndicator)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Button {
                    navigateChapter(offset: 1)
                } label: {
                    Image(systemName: "chevron.right")
                }
                .disabled(txtBook == nil || currentChapterIndex >= (txtBook?.chapters.count ?? 1) - 1)
            }
        }
        .onAppear {
            loadContent()
        }
        .onKeyPress(.leftArrow) {
            navigateChapter(offset: -1)
            return .handled
        }
        .onKeyPress(.rightArrow) {
            navigateChapter(offset: 1)
            return .handled
        }
    }

    // MARK: - Bottom Bar

    private var bottomBar: some View {
        HStack {
            if let txtBook, !txtBook.chapters.isEmpty {
                Text(txtBook.chapters[currentChapterIndex].title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            Text(overallProgress)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }

    private var chapterIndicator: String {
        guard let txtBook else { return "" }
        return "\(currentChapterIndex + 1)/\(txtBook.chapters.count)"
    }

    private var overallProgress: String {
        guard let txtBook, !txtBook.chapters.isEmpty else { return "0%" }
        let chapterWeight = 1.0 / Double(txtBook.chapters.count)
        let pct = (Double(currentChapterIndex) + scrollPercentage / 100.0) * chapterWeight * 100.0
        return "\(Int(pct))%"
    }

    // MARK: - Loading

    private func loadContent() {
        isLoading = true
        errorMessage = nil

        DispatchQueue.global(qos: .userInitiated).async {
            let loaded = TxtContentStore.shared.loadIfNeeded(bookId: book.id, fileURL: book.fileURL)

            DispatchQueue.main.async {
                guard let loaded else {
                    errorMessage = "Failed to load text file. The file may have been moved or deleted."
                    isLoading = false
                    return
                }

                txtBook = loaded
                renderCurrentChapter()
                isLoading = false
            }
        }
    }

    private func renderCurrentChapter() {
        guard let txtBook, currentChapterIndex < txtBook.chapters.count else { return }

        let chapter = txtBook.chapters[currentChapterIndex]
        let renderer = DomRenderer(
            fontSize: fontSize,
            lineSpacing: lineSpacing,
            fontName: fontName
        )

        // Convert TXT chapter to DomNodes for consistent rendering
        let nodes = Self.txtChapterToDomNodes(chapter)
        attributedContent = renderer.render(nodes: nodes)
    }

    private func navigateChapter(offset: Int) {
        guard let txtBook else { return }
        let newIndex = currentChapterIndex + offset
        guard newIndex >= 0, newIndex < txtBook.chapters.count else { return }

        currentChapterIndex = newIndex
        renderCurrentChapter()
    }

    // MARK: - TXT to DomNode Conversion

    /// Convert a TXT chapter into DomNodes for rendering through the existing DomRenderer.
    static func txtChapterToDomNodes(_ chapter: TxtContentStore.TxtChapter) -> [DomNode] {
        var nodes: [DomNode] = []

        // Chapter title as heading
        nodes.append(DomNode(
            nodeType: .heading(level: 2),
            children: [DomNode(nodeType: .text, text: chapter.title)]
        ))

        // Split content into paragraphs by blank lines or line breaks
        let paragraphs = chapter.content
            .components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }

        var currentParagraph = ""

        for line in paragraphs {
            if line.isEmpty {
                // Blank line — flush current paragraph
                if !currentParagraph.isEmpty {
                    nodes.append(DomNode(
                        nodeType: .paragraph,
                        children: [DomNode(nodeType: .text, text: currentParagraph)]
                    ))
                    currentParagraph = ""
                }
            } else {
                if !currentParagraph.isEmpty {
                    currentParagraph += "\n"
                }
                currentParagraph += line
            }
        }

        // Flush remaining
        if !currentParagraph.isEmpty {
            nodes.append(DomNode(
                nodeType: .paragraph,
                children: [DomNode(nodeType: .text, text: currentParagraph)]
            ))
        }

        return nodes
    }
}
