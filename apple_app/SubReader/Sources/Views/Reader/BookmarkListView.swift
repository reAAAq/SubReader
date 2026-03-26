// BookmarkListView — Displays all bookmarks for the current book.

import SwiftUI
import ReaderModels

struct BookmarkListView: View {
    @ObservedObject var manager: BookmarkManager
    var onSelectBookmark: ((Bookmark) -> Void)?

    var body: some View {
        Group {
            if manager.bookmarks.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "bookmark")
                        .font(.title)
                        .foregroundStyle(.secondary)
                    Text("No Bookmarks")
                        .font(.headline)
                        .foregroundStyle(.secondary)
                    Text("Press ⌘B to bookmark the current page")
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
                                Text(bookmark.title ?? "Untitled Bookmark")
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
                            Button("Delete", role: .destructive) {
                                manager.deleteBookmark(id: bookmark.id)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Bookmarks")
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}
