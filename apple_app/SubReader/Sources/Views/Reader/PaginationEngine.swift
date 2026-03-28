// PaginationEngine — CoreText-based pagination engine for splitting attributed text into pages.
//
// Uses CTFramesetter + CTFrame to calculate how much text fits on each page,
// producing an array of PageSlice objects with character ranges and substrings.

import AppKit
import CoreText
import Foundation

/// Represents a single page of content after pagination.
struct PageSlice {
    /// Zero-based page index within the chapter.
    let pageIndex: Int
    /// Character range in the original NSAttributedString.
    let textRange: NSRange
    /// The attributed substring for this page.
    let attributedSubstring: NSAttributedString
}

/// CoreText-based pagination engine that splits an NSAttributedString into fixed-size pages.
final class PaginationEngine {

    /// Page insets (padding inside each page).
    struct PageInsets {
        let top: CGFloat
        let bottom: CGFloat
        let left: CGFloat
        let right: CGFloat

        static let `default` = PageInsets(top: 32, bottom: 48, left: 40, right: 40)
    }

    // MARK: - Paginate

    /// Split the given attributed string into pages that fit within the specified size.
    ///
    /// - Parameters:
    ///   - attributedString: The full chapter content.
    ///   - pageSize: The available size for each page (width × height).
    ///   - insets: Padding inside each page.
    /// - Returns: An array of `PageSlice` objects, one per page.
    static func paginate(
        attributedString: NSAttributedString,
        pageSize: CGSize,
        insets: PageInsets = .default
    ) -> [PageSlice] {
        guard attributedString.length > 0 else {
            return []
        }

        // Calculate the text area after applying insets
        let textWidth = pageSize.width - insets.left - insets.right
        let textHeight = pageSize.height - insets.top - insets.bottom

        guard textWidth > 0, textHeight > 0 else {
            return []
        }

        let framesetter = CTFramesetterCreateWithAttributedString(attributedString as CFAttributedString)
        var pages: [PageSlice] = []
        var currentIndex = 0
        let totalLength = attributedString.length

        while currentIndex < totalLength {
            // Create a path for the text area
            let textRect = CGRect(x: 0, y: 0, width: textWidth, height: textHeight)
            let path = CGPath(rect: textRect, transform: nil)

            // Calculate how much text fits in this frame
            let rangeToFit = CFRange(location: currentIndex, length: totalLength - currentIndex)
            var fitRange = CFRange(location: 0, length: 0)

            CTFramesetterCreateFrame(framesetter, rangeToFit, path, nil)
            CTFramesetterSuggestFrameSizeWithConstraints(
                framesetter,
                rangeToFit,
                nil,
                CGSize(width: textWidth, height: textHeight),
                &fitRange
            )

            // Ensure we make progress (at least 1 character per page)
            if fitRange.length <= 0 {
                fitRange.length = 1
            }

            let nsRange = NSRange(location: currentIndex, length: fitRange.length)
            let substring = attributedString.attributedSubstring(from: nsRange)

            let page = PageSlice(
                pageIndex: pages.count,
                textRange: nsRange,
                attributedSubstring: substring
            )
            pages.append(page)

            currentIndex += fitRange.length
        }

        return pages
    }

    // MARK: - Position Lookup

    /// Find the page index that contains the given character offset.
    ///
    /// - Parameters:
    ///   - characterOffset: The character offset to locate.
    ///   - pages: The paginated page slices.
    /// - Returns: The page index containing the offset, or 0 if not found.
    static func pageIndex(forCharacterOffset characterOffset: Int, in pages: [PageSlice]) -> Int {
        for page in pages {
            let rangeEnd = page.textRange.location + page.textRange.length
            if characterOffset >= page.textRange.location && characterOffset < rangeEnd {
                return page.pageIndex
            }
        }
        // If offset is beyond all pages, return the last page
        if !pages.isEmpty && characterOffset >= pages.last!.textRange.location {
            return pages.count - 1
        }
        return 0
    }

    /// Get the character offset for the start of a given page.
    ///
    /// - Parameters:
    ///   - pageIndex: The page index.
    ///   - pages: The paginated page slices.
    /// - Returns: The character offset at the start of the page, or 0 if invalid.
    static func characterOffset(forPageIndex pageIndex: Int, in pages: [PageSlice]) -> Int {
        guard pageIndex >= 0, pageIndex < pages.count else { return 0 }
        return pages[pageIndex].textRange.location
    }
}
