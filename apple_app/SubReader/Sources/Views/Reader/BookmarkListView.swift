// BookmarkListView — Displays all bookmarks for the current book.

import SwiftUI
import ReaderModels

struct BookmarkListView: View {
    @ObservedObject var manager: BookmarkManager
    @ObservedObject private var languageManager = LanguageManager.shared
    var onSelectBookmark: ((Bookmark) -> Void)?

    var body: some View {
        Group {
            if manager.bookmarks.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "bookmark")
                        .font(.title)
                        .foregroundStyle(.secondary)
                    Text(L("bookmarks.noBookmarks"))
                        .font(.headline)
                        .foregroundStyle(.secondary)
                    Text(L("bookmarks.hint"))
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(manager.bookmarks) { bookmark in
                        Button {
                            onSelectBookmark?(bookmark)
                        } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(bookmark.title ?? L("bookmarks.untitled"))
                                    .font(.body)
                                    .lineLimit(1)

                                Text(formatDate(bookmark.createdAt))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            .padding(.vertical, 2)
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button(L("bookmarks.delete"), role: .destructive) {
                                manager.deleteBookmark(id: bookmark.id)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle(L("bookmarks.title"))
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}
