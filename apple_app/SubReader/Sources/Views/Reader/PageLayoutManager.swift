// PageLayoutManager — Observable layout manager for dual/single page switching.
//
// Monitors available content width and provides a debounced `isDualPage` flag.
// Used by ReaderView and TxtReaderView to decide rendering mode.

import SwiftUI
import Combine

/// Threshold width (in points) above which dual-page mode is activated.
private let dualPageThreshold: CGFloat = 1200

/// Debounce interval to avoid rapid toggling near the threshold.
private let debounceInterval: TimeInterval = 0.3

/// Observable object that tracks whether the reader should use dual-page layout.
@MainActor
final class PageLayoutManager: ObservableObject {

    /// Whether dual-page mode is currently active.
    @Published private(set) var isDualPage: Bool = false

    /// Raw width value before debounce.
    private var rawWidth: CGFloat = 0
    private var debounceTask: DispatchWorkItem?
    private var layoutMode: ReaderPageLayoutMode = .automatic

    /// Update the available content width. Call this from GeometryReader.
    /// - Parameter width: The available width for the reading area (excluding TOC sidebar).
    func updateWidth(_ width: CGFloat) {
        rawWidth = width
        applyLayout(animated: true)
    }

    /// Force an immediate layout evaluation without debounce.
    /// Useful on first appearance.
    func evaluateImmediately(_ width: CGFloat) {
        rawWidth = width
        applyLayout(animated: false)
    }

    /// Apply a user-selected layout mode.
    func setLayoutMode(_ mode: ReaderPageLayoutMode, animated: Bool = true) {
        layoutMode = mode
        applyLayout(animated: animated)
    }

    private func desiredIsDualPage() -> Bool {
        switch layoutMode {
        case .automatic:
            return rawWidth >= dualPageThreshold
        case .single:
            return false
        case .dual:
            return true
        }
    }

    private func applyLayout(animated: Bool) {
        let newIsDual = desiredIsDualPage()

        if !animated || layoutMode != .automatic {
            debounceTask?.cancel()
            debounceTask = nil
            if isDualPage != newIsDual {
                if animated {
                    withAnimation(.easeInOut(duration: 0.25)) {
                        isDualPage = newIsDual
                    }
                } else {
                    isDualPage = newIsDual
                }
            }
            return
        }

        if newIsDual == isDualPage {
            debounceTask?.cancel()
            debounceTask = nil
            return
        }

        debounceTask?.cancel()
        let task = DispatchWorkItem { [weak self] in
            guard let self else { return }
            if self.isDualPage != newIsDual {
                withAnimation(.easeInOut(duration: 0.25)) {
                    self.isDualPage = newIsDual
                }
            }
        }
        debounceTask = task
        DispatchQueue.main.asyncAfter(deadline: .now() + debounceInterval, execute: task)
    }
}

// MARK: - SwiftUI View Modifier

/// A view modifier that injects a PageLayoutManager and monitors geometry changes.
struct PageLayoutModifier: ViewModifier {
    @ObservedObject var layoutManager: PageLayoutManager
    /// Optional width to subtract (e.g. TOC sidebar width) before evaluating.
    var tocWidth: CGFloat = 0

    func body(content: Content) -> some View {
        content
            .background(
                GeometryReader { geo in
                    Color.clear
                        .onAppear {
                            layoutManager.evaluateImmediately(geo.size.width - tocWidth)
                        }
                        .onChange(of: geo.size.width) { _, newWidth in
                            layoutManager.updateWidth(newWidth - tocWidth)
                        }
                }
            )
    }
}

extension View {
    /// Attach a PageLayoutManager that monitors the view's width for dual-page switching.
    func trackPageLayout(manager: PageLayoutManager, tocWidth: CGFloat = 0) -> some View {
        modifier(PageLayoutModifier(layoutManager: manager, tocWidth: tocWidth))
    }
}
