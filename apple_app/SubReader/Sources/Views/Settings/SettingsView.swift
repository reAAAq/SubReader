// SettingsView — macOS Settings panel for reading preferences.

import SwiftUI

struct SettingsView: View {
    @ObservedObject private var preferences = ReadingPreferences.shared
    @Environment(\.colorScheme) private var systemColorScheme

    var body: some View {
        TabView {
            readingTab
                .tabItem {
                    Label("Reading", systemImage: "book")
                }

            appearanceTab
                .tabItem {
                    Label("Appearance", systemImage: "paintbrush")
                }
        }
        .frame(width: 450, height: 350)
    }

    // MARK: - Reading Tab

    private var readingTab: some View {
        Form {
            Section("Font") {
                Picker("Font Family", selection: $preferences.fontName) {
                    Text("System Default").tag("System")
                    Divider()
                    ForEach(availableFonts, id: \.self) { font in
                        Text(font).tag(font)
                    }
                }

                HStack {
                    Text("Font Size")
                    Spacer()
                    Slider(value: $preferences.fontSize, in: 12...36, step: 1)
                        .frame(width: 200)
                    Text("\(Int(preferences.fontSize))pt")
                        .monospacedDigit()
                        .frame(width: 40)
                }

                HStack {
                    Text("Line Spacing")
                    Spacer()
                    Slider(value: $preferences.lineSpacing, in: 1.0...2.5, step: 0.1)
                        .frame(width: 200)
                    Text(String(format: "%.1f", preferences.lineSpacing))
                        .monospacedDigit()
                        .frame(width: 40)
                }
            }

            Section("Preview") {
                Text("The quick brown fox jumps over the lazy dog.")
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
            Section("Theme") {
                Picker("Reading Theme", selection: $preferences.themeType) {
                    ForEach(ReadingThemeType.allCases) { theme in
                        Text(theme.rawValue).tag(theme)
                    }
                }
                .pickerStyle(.segmented)

                Toggle("Follow System Appearance", isOn: $preferences.followSystemAppearance)
            }

            Section("Theme Preview") {
                HStack(spacing: 12) {
                    ForEach(ReadingThemeType.allCases.filter { $0 != .custom }) { themeType in
                        themePreview(themeType)
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

            Text(type.rawValue)
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
