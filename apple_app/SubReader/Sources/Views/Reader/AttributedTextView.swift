// AttributedTextView — NSViewRepresentable wrapper for NSTextView.
//
// Uses native CoreText rendering for maximum performance.
// Reports scroll position for progress tracking.

import SwiftUI
import AppKit

/// NSTextView wrapper for high-performance attributed string rendering.
struct AttributedTextView: NSViewRepresentable {
    let attributedString: NSAttributedString
    var onScroll: ((Double) -> Void)?

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        let textView = scrollView.documentView as! NSTextView

        // Configure for reading
        textView.isEditable = false
        textView.isSelectable = true
        textView.drawsBackground = true
        textView.backgroundColor = .textBackgroundColor
        textView.textContainerInset = NSSize(width: 40, height: 24)
        textView.isAutomaticLinkDetectionEnabled = true
        textView.isRichText = true

        // Performance: disable undo for read-only
        textView.allowsUndo = false

        // Set up scroll notification
        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            context.coordinator,
            selector: #selector(Coordinator.scrollViewDidScroll(_:)),
            name: NSView.boundsDidChangeNotification,
            object: scrollView.contentView
        )

        context.coordinator.textView = textView
        context.coordinator.scrollView = scrollView

        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView else { return }

        // Only update if content actually changed (avoid unnecessary re-renders)
        if textView.attributedString() != attributedString {
            textView.textStorage?.setAttributedString(attributedString)
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onScroll: onScroll)
    }

    class Coordinator: NSObject {
        var textView: NSTextView?
        var scrollView: NSScrollView?
        var onScroll: ((Double) -> Void)?

        init(onScroll: ((Double) -> Void)?) {
            self.onScroll = onScroll
        }

        @objc func scrollViewDidScroll(_ notification: Notification) {
            guard let scrollView = scrollView,
                  let documentView = scrollView.documentView else { return }

            let contentHeight = documentView.frame.height
            let visibleHeight = scrollView.contentView.bounds.height
            let scrollOffset = scrollView.contentView.bounds.origin.y

            guard contentHeight > visibleHeight else { return }

            let percentage = (scrollOffset / (contentHeight - visibleHeight)) * 100.0
            let clamped = min(max(percentage, 0), 100)
            onScroll?(clamped)
        }
    }
}
