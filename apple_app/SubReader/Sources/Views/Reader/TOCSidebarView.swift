// TOCSidebarView — Table of Contents sidebar with tree navigation.

import SwiftUI
import ReaderModels

struct TOCSidebarView: View {
    let tocEntries: [TocEntry]
    let currentChapterHref: String?
    var onSelectChapter: ((TocEntry) -> Void)?

    var body: some View {
        List {
            ForEach(tocEntries) { entry in
                TOCRowView(
                    entry: entry,
                    currentChapterHref: currentChapterHref,
                    onSelectChapter: onSelectChapter
                )
            }
        }
        .listStyle(.sidebar)
        .frame(minWidth: 200)
    }
}

/// Individual TOC row — extracted to its own struct to avoid recursive opaque type issues.
private struct TOCRowView: View {
    let entry: TocEntry
    let currentChapterHref: String?
    var onSelectChapter: ((TocEntry) -> Void)?

    var body: some View {
        if entry.children.isEmpty {
            Button {
                onSelectChapter?(entry)
            } label: {
                tocLabel
            }
            .buttonStyle(.plain)
        } else {
            DisclosureGroup {
                ForEach(entry.children) { child in
                    TOCRowView(
                        entry: child,
                        currentChapterHref: currentChapterHref,
                        onSelectChapter: onSelectChapter
                    )
                }
            } label: {
                Button {
                    onSelectChapter?(entry)
                } label: {
                    tocLabel
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var isCurrent: Bool {
        currentChapterHref == entry.href
    }

    private var tocLabel: some View {
        HStack {
            Text(entry.title)
                .font(.body)
                .fontWeight(isCurrent ? .semibold : .regular)
                .foregroundStyle(isCurrent ? .primary : .secondary)

            Spacer()

            if isCurrent {
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(Color.accentColor)
            }
        }
        .padding(.leading, CGFloat(entry.level) * 12)
    }
}
