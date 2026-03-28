// ReadingPreferences — User reading preferences with @AppStorage persistence.

import SwiftUI
import Combine

/// Persisted page layout preference for the reader.
enum ReaderPageLayoutMode: String, CaseIterable, Identifiable {
    case automatic
    case single
    case dual

    var id: String { rawValue }
}

/// Observable reading preferences that trigger re-renders on change.
final class ReadingPreferences: ObservableObject {

    // MARK: - Published Properties

    @AppStorage("fontSize") var fontSize: Double = 16 {
        willSet { objectWillChange.send() }
    }

    @AppStorage("lineSpacing") var lineSpacing: Double = 1.5 {
        willSet { objectWillChange.send() }
    }

    @AppStorage("fontName") var fontName: String = "System" {
        willSet { objectWillChange.send() }
    }

    @AppStorage("themeType") var themeType: ReadingThemeType = .light {
        willSet { objectWillChange.send() }
    }

    @AppStorage("followSystemAppearance") var followSystemAppearance: Bool = true {
        willSet { objectWillChange.send() }
    }

    @AppStorage("pageLayoutMode") private var pageLayoutModeRawValue: String = ReaderPageLayoutMode.automatic.rawValue {
        willSet { objectWillChange.send() }
    }

    // MARK: - Computed

    var currentTheme: ReadingTheme {
        ReadingTheme.from(themeType)
    }

    var pageLayoutMode: ReaderPageLayoutMode {
        get { ReaderPageLayoutMode(rawValue: pageLayoutModeRawValue) ?? .automatic }
        set { pageLayoutModeRawValue = newValue.rawValue }
    }

    /// Hash for cache invalidation when text appearance changes.
    var themeHash: Int {
        var hasher = Hasher()
        hasher.combine(fontSize)
        hasher.combine(lineSpacing)
        hasher.combine(fontName)
        hasher.combine(themeType)
        return hasher.finalize()
    }

    // MARK: - Singleton

    @MainActor static let shared = ReadingPreferences()
}

// Make ReadingThemeType work with @AppStorage
extension ReadingThemeType: RawRepresentable {
    // Already RawRepresentable via String
}
