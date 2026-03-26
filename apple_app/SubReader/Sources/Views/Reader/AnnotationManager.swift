// AnnotationManager — Annotation management with highlight rendering support.

import Foundation
import ReaderModels
import ReaderBridge

/// Manages annotations (highlights + notes) for the current book.
@MainActor
final class AnnotationManager: ObservableObject {

    @Published var annotations: [Annotation] = []

    private let engine: any ReaderEngineProtocol
    private let bookId: String

    init(engine: any ReaderEngineProtocol, bookId: String) {
        self.engine = engine
        self.bookId = bookId
        loadAnnotations()
    }

    /// Add a highlight annotation.
    func addAnnotation(cfiStart: String, cfiEnd: String, colorRgba: String, note: String? = nil) {
        let annotation = Annotation(
            id: UUID().uuidString,
            bookId: bookId,
            cfiStart: cfiStart,
            cfiEnd: cfiEnd,
            colorRgba: colorRgba,
            note: note,
            createdAt: UInt64(Date().timeIntervalSince1970)
        )

        let result = engine.addAnnotation(annotation)
        if case .success = result {
            annotations.append(annotation)
        }
    }

    /// Delete an annotation by ID.
    func deleteAnnotation(id: String) {
        let hlcTs = UInt64(Date().timeIntervalSince1970)
        let result = engine.deleteAnnotation(id: id, hlcTs: hlcTs)
        if case .success = result {
            annotations.removeAll { $0.id == id }
        }
    }

    /// Get annotations for a specific CFI range (for rendering highlights).
    func annotationsInRange(cfiStart: String, cfiEnd: String) -> [Annotation] {
        // Simplified: return all annotations that overlap with the range
        annotations.filter { ann in
            ann.cfiStart <= cfiEnd && ann.cfiEnd >= cfiStart
        }
    }

    /// Reload annotations from engine.
    func loadAnnotations() {
        let result = engine.listAnnotations(bookId: bookId)
        if case .success(let list) = result {
            annotations = list
        }
    }
}
