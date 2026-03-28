// PageRenderView — Single-page renderer using NSTextView (non-scrollable).
//
// Renders a single page slice of attributed text with no scrolling.
// Supports theme-based background and text colors, and displays a page number at the bottom.

import SwiftUI
import AppKit

/// A non-scrollable page view that renders a single page of attributed text.
struct PageRenderView: NSViewRepresentable {
    let attributedString: NSAttributedString
    let backgroundColor: NSColor
    let pageNumber: Int?
    let pageNumberColor: NSColor

    func makeNSView(context: Context) -> PageRenderNSView {
        let view = PageRenderNSView()
        view.updateContent(
            attributedString,
            backgroundColor: backgroundColor,
            pageNumber: pageNumber,
            pageNumberColor: pageNumberColor
        )
        return view
    }

    func updateNSView(_ nsView: PageRenderNSView, context: Context) {
        nsView.updateContent(
            attributedString,
            backgroundColor: backgroundColor,
            pageNumber: pageNumber,
            pageNumberColor: pageNumberColor
        )
    }
}

/// Custom NSView that renders attributed text without scrolling, with a page number at the bottom.
class PageRenderNSView: NSView {

    private var textView: NSTextView!
    private var pageNumberLabel: NSTextField!
    private var currentContent: NSAttributedString = NSAttributedString()

    /// Padding inside the text area.
    private let horizontalPadding: CGFloat = 40
    private let topPadding: CGFloat = 32
    private let bottomPadding: CGFloat = 48

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setupViews()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setupViews()
    }

    private func setupViews() {
        wantsLayer = true

        // Text view — non-scrollable, non-editable
        textView = NSTextView(frame: .zero)
        textView.isEditable = false
        textView.isSelectable = true
        textView.drawsBackground = false
        textView.isRichText = true
        textView.allowsUndo = false
        textView.isVerticallyResizable = false
        textView.isHorizontallyResizable = false
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = true
        textView.textContainerInset = .zero
        addSubview(textView)

        // Page number label
        pageNumberLabel = NSTextField(labelWithString: "")
        pageNumberLabel.alignment = .center
        pageNumberLabel.font = .systemFont(ofSize: 11, weight: .regular)
        pageNumberLabel.textColor = .secondaryLabelColor
        pageNumberLabel.drawsBackground = false
        pageNumberLabel.isBezeled = false
        pageNumberLabel.isEditable = false
        addSubview(pageNumberLabel)
    }

    override func layout() {
        super.layout()

        let textWidth = bounds.width - horizontalPadding * 2
        let textHeight = bounds.height - topPadding - bottomPadding

        if textWidth > 0, textHeight > 0 {
            textView.frame = NSRect(
                x: horizontalPadding,
                y: bottomPadding,
                width: textWidth,
                height: textHeight
            )
            textView.textContainer?.containerSize = NSSize(width: textWidth, height: textHeight)
        }

        // Page number at the bottom center
        let labelHeight: CGFloat = 20
        pageNumberLabel.frame = NSRect(
            x: 0,
            y: (bottomPadding - labelHeight) / 2,
            width: bounds.width,
            height: labelHeight
        )
    }

    func updateContent(
        _ attributedString: NSAttributedString,
        backgroundColor: NSColor,
        pageNumber: Int?,
        pageNumberColor: NSColor
    ) {
        // Update background
        layer?.backgroundColor = backgroundColor.cgColor

        // Update text content
        if currentContent != attributedString {
            currentContent = attributedString
            textView.textStorage?.setAttributedString(attributedString)
        }

        // Update page number
        if let num = pageNumber {
            pageNumberLabel.stringValue = "\(num)"
            pageNumberLabel.textColor = pageNumberColor
            pageNumberLabel.isHidden = false
        } else {
            pageNumberLabel.isHidden = true
        }
    }
}
