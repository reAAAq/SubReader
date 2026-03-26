// String+Localized — Convenience helpers for localized string access.
//
// Provides a global `L()` function and String extension for easy localization.

import Foundation

/// Global convenience function to get a localized string.
/// Usage: `L("sidebar.allBooks")` or `L("reader.chapter", 5)`
@MainActor
func L(_ key: String) -> String {
    LanguageManager.shared.localizedString(forKey: key)
}

/// Global convenience function to get a localized string with format arguments.
@MainActor
func L(_ key: String, _ args: CVarArg...) -> String {
    let format = LanguageManager.shared.localizedString(forKey: key)
    return String(format: format, arguments: args)
}

extension String {
    /// Returns the localized version of this string key.
    @MainActor
    var localized: String {
        LanguageManager.shared.localizedString(forKey: self)
    }
}
