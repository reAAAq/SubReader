// TOCSidebarView — Table of Contents sidebar with tree navigation.

import SwiftUI
import ReaderModels

struct TOCSidebarView: View {
    let tocEntries: [TocEntry]
    let currentChapterHref: String?
    var onSelectChapter: ((TocEntry) -> Void)?

    var body: some View {
        if tocEntries.isEmpty {
            VStack {
                Spacer()
                Text("No table of contents")
                    .foregroundStyle(.secondary)
                    .font(.subheadline)
                Spacer()
            }
            .frame(minWidth: 200)
        } else {
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
        guard let currentHref = currentChapterHref else { return false }
        // Strip fragment identifiers before comparing
        let baseCurrentHref = currentHref.components(separatedBy: "#").first ?? currentHref
        let baseEntryHref = entry.href.components(separatedBy: "#").first ?? entry.href
        return baseCurrentHref == baseEntryHref
            || baseCurrentHref.hasSuffix(baseEntryHref)
            || baseEntryHref.hasSuffix(baseCurrentHref)
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
