// LanguageManager — Manages app language preference and localized string loading.
//
// Supports three modes: follow system, force Chinese, force English.
// Uses custom Bundle loading to enable in-app language switching without restart.

import SwiftUI
import Combine

/// Available language options for the app.
enum AppLanguage: String, CaseIterable, Identifiable {
    case system = "system"
    case zhHans = "zh-Hans"
    case en = "en"

    var id: String { rawValue }

    /// Display name for the language option (always shown in its own language).
    var displayName: String {
        switch self {
        case .system:
            return LanguageManager.shared.localizedString(forKey: "language.system")
        case .zhHans:
            return "简体中文"
        case .en:
            return "English"
        }
    }
}

/// Observable language manager that triggers UI re-renders on language change.
final class LanguageManager: ObservableObject {

    // MARK: - Singleton

    @MainActor static let shared = LanguageManager()

    // MARK: - Published Properties

    @AppStorage("appLanguage") var appLanguage: AppLanguage = .system {
        willSet { objectWillChange.send() }
        didSet { updateBundle() }
    }

    // MARK: - Private Properties

    /// The bundle used to load localized strings based on current language preference.
    private(set) var localizedBundle: Bundle = .module

    // MARK: - Init

    init() {
        updateBundle()
    }

    // MARK: - Public Methods

    /// Returns the localized string for the given key.
    func localizedString(forKey key: String) -> String {
        let value = localizedBundle.localizedString(forKey: key, value: nil, table: nil)
        // If the key was not found (returns the key itself), fall back to English bundle
        if value == key {
            if let enPath = Bundle.module.path(forResource: "en", ofType: "lproj"),
               let enBundle = Bundle(path: enPath) {
                return enBundle.localizedString(forKey: key, value: key, table: nil)
            }
        }
        return value
    }

    /// Returns the localized string with format arguments.
    func localizedString(forKey key: String, _ args: CVarArg...) -> String {
        let format = localizedString(forKey: key)
        return String(format: format, arguments: args)
    }

    // MARK: - Private Methods

    /// Resolves the effective language code based on user preference.
    private var effectiveLanguageCode: String {
        switch appLanguage {
        case .system:
            return resolveSystemLanguage()
        case .zhHans:
            return "zh-Hans"
        case .en:
            return "en"
        }
    }

    /// Determines the system language, falling back to English for unsupported languages.
    private func resolveSystemLanguage() -> String {
        let preferred = Locale.preferredLanguages.first ?? "en"
        // Check if system language is Chinese (any variant)
        if preferred.hasPrefix("zh-Hans") || preferred.hasPrefix("zh_Hans") {
            return "zh-Hans"
        }
        if preferred.hasPrefix("zh-Hant") || preferred.hasPrefix("zh_Hant") || preferred.hasPrefix("zh") {
            // Traditional Chinese falls back to Simplified Chinese
            return "zh-Hans"
        }
        // All other languages fall back to English
        return "en"
    }

    /// Updates the localized bundle based on the current language preference.
    private func updateBundle() {
        let langCode = effectiveLanguageCode
        if let path = Bundle.module.path(forResource: langCode, ofType: "lproj"),
           let bundle = Bundle(path: path) {
            localizedBundle = bundle
        } else if let path = Bundle.module.path(forResource: "en", ofType: "lproj"),
                  let bundle = Bundle(path: path) {
            // Fall back to English
            localizedBundle = bundle
        } else {
            localizedBundle = .module
        }
    }
}

// MARK: - AppLanguage + RawRepresentable for @AppStorage

extension AppLanguage: RawRepresentable {
    // Already RawRepresentable via String
}
