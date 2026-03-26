// AnnotationListView — Displays all annotations for the current book.

import SwiftUI
import ReaderModels

struct AnnotationListView: View {
    @ObservedObject var manager: AnnotationManager
    var onSelectAnnotation: ((Annotation) -> Void)?

    var body: some View {
        Group {
            if manager.annotations.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "highlighter")
                        .font(.title)
                        .foregroundStyle(.secondary)
                    Text("No Annotations")
                        .font(.headline)
                        .foregroundStyle(.secondary)
                    Text("Select text to add highlights and notes")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(manager.annotations) { annotation in
                        Button {
                            onSelectAnnotation?(annotation)
                        } label: {
                            HStack(spacing: 8) {
                                // Color indicator
                                Circle()
                                    .fill(colorFromRgba(annotation.colorRgba))
                                    .frame(width: 12, height: 12)

                                VStack(alignment: .leading, spacing: 4) {
                                    if let note = annotation.note, !note.isEmpty {
                                        Text(note)
                                            .font(.body)
                                            .lineLimit(2)
                                    } else {
                                        Text("Highlight")
                                            .font(.body)
                                            .foregroundStyle(.secondary)
                                    }

                                    Text(formatDate(annotation.createdAt))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .padding(.vertical, 2)
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button("Delete", role: .destructive) {
                                manager.deleteAnnotation(id: annotation.id)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle("Annotations")
    }

    private func colorFromRgba(_ rgba: String) -> Color {
        // Parse "#RRGGBBAA" format
        var hex = rgba.trimmingCharacters(in: .init(charactersIn: "#"))
        if hex.count == 6 { hex += "FF" }
        guard hex.count == 8, let value = UInt64(hex, radix: 16) else {
            return .yellow
        }
        let r = Double((value >> 24) & 0xFF) / 255.0
        let g = Double((value >> 16) & 0xFF) / 255.0
        let b = Double((value >> 8) & 0xFF) / 255.0
        let a = Double(value & 0xFF) / 255.0
        return Color(red: r, green: g, blue: b, opacity: a)
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}
