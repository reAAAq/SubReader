// ReadingTheme — Theme model for reading appearance.

import SwiftUI

/// Available reading themes.
enum ReadingThemeType: String, CaseIterable, Identifiable {
    case light = "Light"
    case dark = "Dark"
    case sepia = "Sepia"
    case custom = "Custom"

    var id: String { rawValue }
}

/// Theme configuration for the reading view.
struct ReadingTheme: Equatable {
    let backgroundColor: Color
    let textColor: Color
    let selectionColor: Color

    static let light = ReadingTheme(
        backgroundColor: .white,
        textColor: .black,
        selectionColor: .accentColor.opacity(0.3)
    )

    static let dark = ReadingTheme(
        backgroundColor: Color(nsColor: .init(white: 0.12, alpha: 1)),
        textColor: Color(nsColor: .init(white: 0.9, alpha: 1)),
        selectionColor: .accentColor.opacity(0.3)
    )

    static let sepia = ReadingTheme(
        backgroundColor: Color(red: 0.96, green: 0.93, blue: 0.87),
        textColor: Color(red: 0.3, green: 0.25, blue: 0.2),
        selectionColor: Color.orange.opacity(0.3)
    )

    static func from(_ type: ReadingThemeType) -> ReadingTheme {
        switch type {
        case .light: return .light
        case .dark: return .dark
        case .sepia: return .sepia
        case .custom: return .light // Customizable later
        }
    }
}
