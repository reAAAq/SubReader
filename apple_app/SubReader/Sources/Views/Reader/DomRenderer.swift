// DomRenderer — Converts DOM tree to NSAttributedString using CoreText.
//
// Protocol-based design for testability. Uses native macOS text rendering
// for maximum performance.

import AppKit
import Foundation
import ReaderModels

// MARK: - Protocol

/// Protocol for DOM rendering engines.
protocol DomRendererProtocol {
    func render(nodes: [DomNode]) -> NSAttributedString
}

// MARK: - Implementation

/// High-performance DOM-to-NSAttributedString renderer.
struct DomRenderer: DomRendererProtocol {

    let fontSize: Double
    let lineSpacing: Double
    let fontName: String
    /// Text color for body text. Defaults to labelColor if not specified.
    let textColor: NSColor

    // MARK: - Computed Styles

    init(
        fontSize: Double,
        lineSpacing: Double,
        fontName: String,
        textColor: NSColor = .labelColor
    ) {
        self.fontSize = fontSize
        self.lineSpacing = lineSpacing
        self.fontName = fontName
        self.textColor = textColor
    }

    private var bodyFont: NSFont {
        if fontName == "System" || fontName.isEmpty {
            return .systemFont(ofSize: fontSize)
        }
        return NSFont(name: fontName, size: fontSize) ?? .systemFont(ofSize: fontSize)
    }

    private var bodyParagraphStyle: NSMutableParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = fontSize * (lineSpacing - 1.0)
        style.paragraphSpacing = fontSize * 0.5
        return style
    }

    // MARK: - Render

    func render(nodes: [DomNode]) -> NSAttributedString {
        let result = NSMutableAttributedString()
        for node in nodes {
            result.append(renderNode(node))
        }
        return result
    }

    // MARK: - Node Rendering

    private func renderNode(_ node: DomNode) -> NSAttributedString {
        switch node.nodeType {
        case .document:
            return renderChildren(node.children)

        case .heading(let level):
            return renderHeading(node, level: level)

        case .paragraph:
            return renderParagraph(node)

        case .text:
            return renderText(node)

        case .image:
            return renderImage(node)

        case .link:
            return renderLink(node)

        case .emphasis:
            return renderInlineStyle(node, trait: .italicFontMask)

        case .strong:
            return renderInlineStyle(node, trait: .boldFontMask)

        case .code:
            return renderCode(node)

        case .blockQuote:
            return renderBlockQuote(node)

        case .list:
            return renderList(node)

        case .listItem:
            return renderListItem(node)

        case .lineBreak:
            return NSAttributedString(string: "\n")

        case .table, .tableRow, .tableCell:
            // Simplified table rendering — just render children with spacing
            return renderChildren(node.children)

        case .span:
            return renderChildren(node.children)
        }
    }

    // MARK: - Heading

    private func renderHeading(_ node: DomNode, level: UInt8) -> NSAttributedString {
        let multiplier: CGFloat
        switch level {
        case 1: multiplier = 1.75
        case 2: multiplier = 1.5
        case 3: multiplier = 1.25
        case 4: multiplier = 1.1
        case 5: multiplier = 1.0
        default: multiplier = 0.9
        }

        let size = CGFloat(fontSize) * multiplier
        let font = NSFont.boldSystemFont(ofSize: size)

        let style = NSMutableParagraphStyle()
        style.paragraphSpacingBefore = size * 0.8
        style.paragraphSpacing = size * 0.4

        let attrs: [NSAttributedString.Key: Any] = [
            .font: font,
            .paragraphStyle: style,
            .foregroundColor: textColor,
        ]

        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)
        result.addAttributes(attrs, range: NSRange(location: 0, length: result.length))
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // MARK: - Paragraph

    private func renderParagraph(_ node: DomNode) -> NSAttributedString {
        let attrs: [NSAttributedString.Key: Any] = [
            .font: bodyFont,
            .paragraphStyle: bodyParagraphStyle,
            .foregroundColor: textColor,
        ]
        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)
        result.addAttributes(attrs, range: NSRange(location: 0, length: result.length))
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // MARK: - Text

    private func renderText(_ node: DomNode) -> NSAttributedString {
        let text = node.text ?? ""
        return NSAttributedString(string: text, attributes: [
            .font: bodyFont,
            .foregroundColor: textColor,
        ])
    }
    // MARK: - Image

    private func renderImage(_ node: DomNode) -> NSAttributedString {
        // Placeholder for images — actual image loading requires EPUB resource extraction
        let attachment = NSTextAttachment()
        let placeholder = NSImage(systemSymbolName: "photo", accessibilityDescription: "Image")
        attachment.image = placeholder
        let result = NSMutableAttributedString(attachment: attachment)
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // MARK: - Link

    private func renderLink(_ node: DomNode) -> NSAttributedString {
        let href = node.attributes.first(where: { $0.first == "href" })?.last ?? ""
        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)

        let attrs: [NSAttributedString.Key: Any] = [
            .foregroundColor: NSColor.linkColor,
            .underlineStyle: NSUnderlineStyle.single.rawValue,
            .link: href,
        ]
        result.addAttributes(attrs, range: NSRange(location: 0, length: result.length))
        return result
    }

    // MARK: - Inline Styles

    private func renderInlineStyle(_ node: DomNode, trait: NSFontTraitMask) -> NSAttributedString {
        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)

        let convertedFont = NSFontManager.shared.convert(bodyFont, toHaveTrait: trait)
        result.addAttribute(.font, value: convertedFont, range: NSRange(location: 0, length: result.length))
        return result
    }

    // MARK: - Code

    private func renderCode(_ node: DomNode) -> NSAttributedString {
        let codeFont = NSFont.monospacedSystemFont(ofSize: fontSize * 0.9, weight: .regular)
        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)

        let attrs: [NSAttributedString.Key: Any] = [
            .font: codeFont,
            .backgroundColor: NSColor.quaternaryLabelColor,
            .foregroundColor: textColor,
        ]
        result.addAttributes(attrs, range: NSRange(location: 0, length: result.length))
        return result
    }

    // MARK: - Block Quote
    private func renderBlockQuote(_ node: DomNode) -> NSAttributedString {
        let style = NSMutableParagraphStyle()
        style.headIndent = 24
        style.firstLineHeadIndent = 24
        style.paragraphSpacing = fontSize * 0.3

        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)

        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont(descriptor: bodyFont.fontDescriptor, size: fontSize) ?? bodyFont,
            .paragraphStyle: style,
            .foregroundColor: textColor.withAlphaComponent(0.7),
        ]
        result.addAttributes(attrs, range: NSRange(location: 0, length: result.length))
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // MARK: - List

    private func renderList(_ node: DomNode) -> NSAttributedString {
        let result = NSMutableAttributedString()
        for (index, child) in node.children.enumerated() {
            if case .list(let ordered) = node.nodeType {
                let bullet = ordered ? "\(index + 1). " : "• "
                result.append(NSAttributedString(string: bullet, attributes: [.font: bodyFont]))
            }
            result.append(renderNode(child))
        }
        return result
    }

    private func renderListItem(_ node: DomNode) -> NSAttributedString {
        let style = NSMutableParagraphStyle()
        style.headIndent = 24
        style.firstLineHeadIndent = 8
        style.paragraphSpacing = fontSize * 0.2

        let result = NSMutableAttributedString()
        let childText = renderChildren(node.children)
        result.append(childText)
        result.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: result.length))
        result.append(NSAttributedString(string: "\n"))
        return result
    }

    // MARK: - Helpers

    private func renderChildren(_ children: [DomNode]) -> NSAttributedString {
        let result = NSMutableAttributedString()
        for child in children {
            result.append(renderNode(child))
        }
        return result
    }
}
