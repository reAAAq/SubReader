// SettingsView — macOS Settings panel for reading preferences.

import SwiftUI

struct SettingsView: View {
    @ObservedObject private var preferences = ReadingPreferences.shared
    @ObservedObject private var languageManager = LanguageManager.shared
    @Environment(\.colorScheme) private var systemColorScheme

    var body: some View {
        TabView {
            readingTab
                .tabItem {
                    Label(L("settings.reading"), systemImage: "book")
                }

            appearanceTab
                .tabItem {
                    Label(L("settings.appearance"), systemImage: "paintbrush")
                }
        }
        .frame(width: 450, height: 400)
    }

    // MARK: - Reading Tab

    private var readingTab: some View {
        Form {
            Section(L("settings.font")) {
                Picker(L("settings.fontFamily"), selection: $preferences.fontName) {
                    Text(L("settings.systemDefault")).tag("System")
                    Divider()
                    ForEach(availableFonts, id: \.self) { font in
                        Text(font).tag(font)
                    }
                }

                HStack {
                    Text(L("settings.fontSize"))
                    Spacer()
                    Slider(value: $preferences.fontSize, in: 12...36, step: 1)
                        .frame(width: 200)
                    Text("\(Int(preferences.fontSize))pt")
                        .monospacedDigit()
                        .frame(width: 40)
                }

                HStack {
                    Text(L("settings.lineSpacing"))
                    Spacer()
                    Slider(value: $preferences.lineSpacing, in: 1.0...2.5, step: 0.1)
                        .frame(width: 200)
                    Text(String(format: "%.1f", preferences.lineSpacing))
                        .monospacedDigit()
                        .frame(width: 40)
                }
            }

            Section(L("settings.preview")) {
                Text(L("settings.previewText"))
                    .font(previewFont)
                    .lineSpacing(preferences.fontSize * (preferences.lineSpacing - 1.0))
                    .padding()
                    .frame(maxWidth: .infinity)
                    .background(preferences.currentTheme.backgroundColor)
                    .foregroundStyle(preferences.currentTheme.textColor)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
        .formStyle(.grouped)
        .padding()
    }

    // MARK: - Appearance Tab

    private var appearanceTab: some View {
        Form {
            Section(L("settings.theme")) {
                Picker(L("settings.readingTheme"), selection: $preferences.themeType) {
                    ForEach(ReadingThemeType.allCases) { theme in
                        Text(theme.displayName).tag(theme)
                    }
                }
                .pickerStyle(.segmented)

                Toggle(L("settings.followSystemAppearance"), isOn: $preferences.followSystemAppearance)
            }

            Section(L("settings.themePreview")) {
                HStack(spacing: 12) {
                    ForEach(ReadingThemeType.allCases.filter { $0 != .custom }) { themeType in
                        themePreview(themeType)
                    }
                }
            }

            Section(L("settings.language")) {
                Picker(L("settings.languageLabel"), selection: $languageManager.appLanguage) {
                    ForEach(AppLanguage.allCases) { lang in
                        Text(lang.displayName).tag(lang)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .padding()
    }

    // MARK: - Components

    private func themePreview(_ type: ReadingThemeType) -> some View {
        let theme = ReadingTheme.from(type)
        return VStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 8)
                .fill(theme.backgroundColor)
                .frame(width: 80, height: 60)
                .overlay {
                    Text("Aa")
                        .foregroundStyle(theme.textColor)
                }
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(
                            preferences.themeType == type ? Color.accentColor : Color.clear,
                            lineWidth: 2
                        )
                )

            Text(type.displayName)
                .font(.caption)
        }
        .onTapGesture {
            preferences.themeType = type
        }
    }

    // MARK: - Helpers

    private var previewFont: Font {
        if preferences.fontName == "System" {
            return .system(size: preferences.fontSize)
        }
        return .custom(preferences.fontName, size: preferences.fontSize)
    }

    private var availableFonts: [String] {
        // Return a curated list of readable fonts
        let preferred = [
            "Georgia", "Palatino", "Times New Roman",
            "Helvetica Neue", "Avenir", "Avenir Next",
            "Charter", "Iowan Old Style", "Baskerville",
            "Menlo", "SF Mono"
        ]
        let available = NSFontManager.shared.availableFontFamilies
        return preferred.filter { available.contains($0) }
    }
}
