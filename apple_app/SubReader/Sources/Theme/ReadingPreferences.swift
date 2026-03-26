// ReadingPreferences — User reading preferences with @AppStorage persistence.

import SwiftUI
import Combine

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

    // MARK: - Computed

    var currentTheme: ReadingTheme {
        ReadingTheme.from(themeType)
    }

    /// Hash for cache invalidation when preferences change.
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
