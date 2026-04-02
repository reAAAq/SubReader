// AccountView — Account management tab in Settings.
//
// Shows login/register forms when logged out, and account info + sync status when logged in.

import SwiftUI
import AppKit
import ReaderBridge

// MARK: - AccountView (Root)

struct AccountView: View {
    @EnvironmentObject private var authService: AuthService
    @EnvironmentObject private var syncService: SyncService
    @ObservedObject private var languageManager = LanguageManager.shared

    var body: some View {
        Group {
            if authService.isLoggedIn {
                LoggedInView()
            } else {
                AuthFormView()
            }
        }
    }
}

// MARK: - Auth Form (Login / Register)

private struct AuthFormView: View {
    @ObservedObject private var languageManager = LanguageManager.shared
    @State private var isRegistering = false
    @State private var showSuccessBanner = false

    var body: some View {
        VStack(spacing: 0) {
            if showSuccessBanner {
                HStack {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Text(L("account.registrationSuccess"))
                        .font(.callout)
                }
                .padding(8)
                .frame(maxWidth: .infinity)
                .background(.green.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .padding(.horizontal)
                .padding(.top, 8)
            }

            if isRegistering {
                RegisterFormView(
                    onSwitchToLogin: {
                        withAnimation { isRegistering = false }
                    },
                    onRegisterSuccess: {
                        withAnimation {
                            isRegistering = false
                            showSuccessBanner = true
                        }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
                            withAnimation { showSuccessBanner = false }
                        }
                    }
                )
            } else {
                LoginFormView(
                    onSwitchToRegister: {
                        withAnimation {
                            isRegistering = true
                            showSuccessBanner = false
                        }
                    }
                )
            }
        }
    }
}

// MARK: - Login Form

private struct LoginFormView: View {
    @EnvironmentObject private var authService: AuthService
    @ObservedObject private var languageManager = LanguageManager.shared
    @State private var credential = ""
    @State private var password = ""
    @State private var showError = false

    var onSwitchToRegister: () -> Void

    private var isFormValid: Bool {
        !credential.isEmpty && password.count >= 8
    }

    var body: some View {
        VStack(spacing: 20) {
            Spacer(minLength: 24)

            // Icon
            Image(systemName: "person.crop.circle")
                .font(.system(size: 56, weight: .thin))
                .foregroundStyle(.secondary)

            // Title
            Text(L("account.signIn"))
                .font(.title2.weight(.semibold))

            // Fields
            VStack(spacing: 12) {
                AppKitTextField(placeholder: L("account.usernameOrEmail"), text: $credential)
                    .frame(height: 28)

                AppKitSecureField(placeholder: L("account.password"), text: $password)
                    .frame(height: 28)
            }
            .padding(.horizontal, 40)
            .padding(.top, 4)

            // Sign-in button
            if authService.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .padding(.top, 4)
            } else {
                Button {
                    Task {
                        let result = await authService.login(credential: credential, password: password)
                        if case .failure = result { showError = true }
                    }
                } label: {
                    Text(L("account.signIn"))
                        .frame(minWidth: 120)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.regular)
                .disabled(!isFormValid)
                .padding(.top, 4)
            }

            Divider()
                .padding(.horizontal, 40)

            // Switch to register
            HStack(spacing: 4) {
                Text(L("account.noAccount"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Button(L("account.register")) {
                    onSwitchToRegister()
                }
                .buttonStyle(.link)
                .font(.callout)
            }

            Spacer(minLength: 24)
        }
        .alert(L("account.loginFailed"), isPresented: $showError) {
            Button(L("error.ok")) { showError = false }
        } message: {
            Text(authService.errorMessage ?? L("account.unknownError"))
        }
    }
}

// MARK: - Register Form

private struct RegisterFormView: View {
    @EnvironmentObject private var authService: AuthService
    @ObservedObject private var languageManager = LanguageManager.shared
    @State private var username = ""
    @State private var email = ""
    @State private var password = ""
    @State private var confirmPassword = ""
    @State private var showError = false

    var onSwitchToLogin: () -> Void
    var onRegisterSuccess: () -> Void

    private var passwordsMatch: Bool {
        password == confirmPassword
    }

    private var isFormValid: Bool {
        !username.isEmpty && !email.isEmpty && password.count >= 8 && passwordsMatch
    }

    var body: some View {
        VStack(spacing: 16) {
            Spacer(minLength: 16)

            // Icon
            Image(systemName: "person.badge.plus")
                .font(.system(size: 48, weight: .thin))
                .foregroundStyle(.secondary)

            // Title
            Text(L("account.createAccount"))
                .font(.title2.weight(.semibold))

            // Fields
            VStack(spacing: 10) {
                AppKitTextField(placeholder: L("account.username"), text: $username)
                    .frame(height: 28)

                AppKitTextField(placeholder: L("account.email"), text: $email)
                    .frame(height: 28)

                AppKitSecureField(placeholder: L("account.passwordMin8"), text: $password)
                    .frame(height: 28)

                AppKitSecureField(placeholder: L("account.confirmPassword"), text: $confirmPassword)
                    .frame(height: 28)

                if !confirmPassword.isEmpty && !passwordsMatch {
                    Text(L("account.passwordsMismatch"))
                        .font(.caption)
                        .foregroundStyle(.red)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.horizontal, 40)
            .padding(.top, 4)

            // Register button
            if authService.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .padding(.top, 4)
            } else {
                Button {
                    Task {
                        let result = await authService.register(
                            username: username,
                            email: email,
                            password: password
                        )
                        switch result {
                        case .success: onRegisterSuccess()
                        case .failure: showError = true
                        }
                    }
                } label: {
                    Text(L("account.register"))
                        .frame(minWidth: 120)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.regular)
                .disabled(!isFormValid)
                .padding(.top, 4)
            }

            Divider()
                .padding(.horizontal, 40)

            // Switch to login
            HStack(spacing: 4) {
                Text(L("account.haveAccount"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Button(L("account.signIn")) {
                    onSwitchToLogin()
                }
                .buttonStyle(.link)
                .font(.callout)
            }

            Spacer(minLength: 16)
        }
        .alert(L("account.registrationFailed"), isPresented: $showError) {
            Button(L("error.ok")) { showError = false }
        } message: {
            Text(authService.errorMessage ?? L("account.unknownError"))
        }
    }
}

// MARK: - Logged In View

private struct LoggedInView: View {
    @EnvironmentObject private var authService: AuthService
    @EnvironmentObject private var syncService: SyncService
    @ObservedObject private var languageManager = LanguageManager.shared
    @State private var showLogoutConfirm = false
    @State private var showChangePassword = false
    @State private var showDevices = false

    var body: some View {
        Form {
            Section {
                HStack {
                    Image(systemName: "person.circle.fill")
                        .font(.system(size: 40))
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(L("account.signedIn"))
                            .font(.headline)
                        Text(
                            L(
                                authService.authState == .authenticated
                                    ? "account.status.active"
                                    : "account.status.needsAttention"
                            )
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Label(L("account.accountSection"), systemImage: "person.circle")
                    .font(.headline)
            }

            Section(L("account.syncSection")) {
                Toggle(isOn: Binding(
                    get: { syncService.autoSyncEnabled },
                    set: { syncService.autoSyncEnabled = $0 }
                )) {
                    Label(L("account.autoSync"), systemImage: "arrow.triangle.2.circlepath.circle")
                }

                if syncService.autoSyncEnabled {
                    SyncStatusView()
                }

                Button {
                    Task { let _ = await syncService.syncNow() }
                } label: {
                    Label(L("account.syncNow"), systemImage: "arrow.triangle.2.circlepath")
                }
                .disabled(!authService.isLoggedIn || syncService.syncState == .syncing)
            }

            Section(L("account.manageSection")) {
                Button {
                    showDevices = true
                } label: {
                    Label(L("account.devices"), systemImage: "desktopcomputer")
                }

                Button {
                    showChangePassword = true
                } label: {
                    Label(L("account.changePassword"), systemImage: "key")
                }

                Button(role: .destructive) {
                    showLogoutConfirm = true
                } label: {
                    Label(L("account.signOut"), systemImage: "rectangle.portrait.and.arrow.right")
                }
            }
        }
        .formStyle(.grouped)
        .padding()
        .confirmationDialog(L("account.signOut"), isPresented: $showLogoutConfirm) {
            Button(L("account.signOut"), role: .destructive) {
                Task { let _ = await authService.logout() }
            }
            Button(L("account.cancel"), role: .cancel) {}
        } message: {
            Text(L("account.signOutConfirmMessage"))
        }
        .sheet(isPresented: $showChangePassword) {
            ChangePasswordView()
        }
        .sheet(isPresented: $showDevices) {
            DeviceListView()
        }
    }
}

// MARK: - Sync Status View

private struct SyncStatusView: View {
    @EnvironmentObject private var syncService: SyncService
    @ObservedObject private var languageManager = LanguageManager.shared

    var body: some View {
        HStack(spacing: 8) {
            statusIcon
            statusText
            Spacer()
        }
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch syncService.syncState {
        case .syncing:
            ProgressView()
                .controlSize(.small)
        case .idle:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .error:
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        case .offline:
            Image(systemName: "cloud.slash")
                .foregroundStyle(.secondary)
        case .dormant:
            if syncService.autoSyncEnabled {
                Image(systemName: "clock.arrow.2.circlepath")
                    .foregroundStyle(.secondary)
            } else {
                Image(systemName: "moon.zzz")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var statusText: Text {
        switch syncService.syncState {
        case .syncing:
            Text(L("account.syncing"))
        case .idle:
            Text(L("account.upToDate"))
        case .error:
            Text(L("account.syncFailed"))
        case .offline:
            Text(L("account.offline"))
        case .dormant:
            if syncService.autoSyncEnabled {
                Text(L("account.syncWaiting"))
            } else {
                Text(L("account.syncPaused"))
            }
        }
    }
}

// MARK: - Change Password View

private struct ChangePasswordView: View {
    @EnvironmentObject private var authService: AuthService
    @ObservedObject private var languageManager = LanguageManager.shared
    @Environment(\.dismiss) private var dismiss
    @State private var oldPassword = ""
    @State private var newPassword = ""
    @State private var confirmPassword = ""
    @State private var showError = false

    private var isFormValid: Bool {
        !oldPassword.isEmpty && newPassword.count >= 8 && newPassword == confirmPassword
    }

    var body: some View {
        VStack(spacing: 16) {
            Text(L("account.changePassword"))
                .font(.headline)

            Form {
                AppKitSecureField(placeholder: L("account.currentPassword"), text: $oldPassword)
                    .frame(maxWidth: .infinity, minHeight: 28)

                AppKitSecureField(placeholder: L("account.newPasswordMin8"), text: $newPassword)
                    .frame(maxWidth: .infinity, minHeight: 28)

                AppKitSecureField(placeholder: L("account.confirmNewPassword"), text: $confirmPassword)
                    .frame(maxWidth: .infinity, minHeight: 28)

                if !confirmPassword.isEmpty && newPassword != confirmPassword {
                    Text(L("account.passwordsMismatch"))
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
            .formStyle(.grouped)

            HStack {
                Button(L("account.cancel")) { dismiss() }
                    .buttonStyle(.bordered)

                if authService.isLoading {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Button(L("account.changePassword")) {
                        Task {
                            let result = await authService.changePassword(
                                oldPassword: oldPassword,
                                newPassword: newPassword
                            )
                            switch result {
                            case .success:
                                dismiss()
                            case .failure:
                                showError = true
                            }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!isFormValid)
                }
            }
        }
        .padding()
        .frame(width: 400, height: 350)
        .alert(L("account.changePasswordFailed"), isPresented: $showError) {
            Button(L("error.ok")) { showError = false }
        } message: {
            Text(authService.errorMessage ?? L("account.unknownError"))
        }
    }
}

// MARK: - Device List View

private struct DeviceListView: View {
    @EnvironmentObject private var authService: AuthService
    @ObservedObject private var languageManager = LanguageManager.shared
    @Environment(\.dismiss) private var dismiss
    @State private var devicesJSON = ""
    @State private var isLoading = true
    @State private var errorMessage: String?

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                Text(L("account.devices"))
                    .font(.headline)
                Spacer()
                Button(L("account.done")) { dismiss() }
                    .buttonStyle(.bordered)
            }

            if isLoading {
                ProgressView(L("account.loadingDevices"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error = errorMessage {
                VStack {
                    Image(systemName: "exclamationmark.triangle")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                    Text(error)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    Text(devicesJSON)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .padding()
        .frame(width: 500, height: 400)
        .task {
            await loadDevices()
        }
    }

    private func loadDevices() async {
        isLoading = true
        let result = await authService.listDevices()
        isLoading = false

        switch result {
        case .success(let json):
            devicesJSON = json
        case .failure(let error):
            errorMessage = error.localizedDescription
        }
    }
}

// MARK: - AppKit Input Wrappers

private struct AppKitTextField: NSViewRepresentable {
    let placeholder: String
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeNSView(context: Context) -> NSTextField {
        let textField = NSTextField(string: text)
        configure(textField, coordinator: context.coordinator)
        return textField
    }

    func updateNSView(_ nsView: NSTextField, context: Context) {
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
        nsView.placeholderString = placeholder
    }

    private func configure(_ textField: NSTextField, coordinator: Coordinator) {
        textField.isEditable = true
        textField.isSelectable = true
        textField.isBordered = true
        textField.isBezeled = true
        textField.bezelStyle = .roundedBezel
        textField.drawsBackground = true
        textField.focusRingType = .default
        textField.placeholderString = placeholder
        textField.delegate = coordinator
        textField.maximumNumberOfLines = 1
        textField.lineBreakMode = .byTruncatingTail
        textField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        @Binding private var text: String

        init(text: Binding<String>) {
            _text = text
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let textField = obj.object as? NSTextField else { return }
            if text != textField.stringValue {
                text = textField.stringValue
            }
        }
    }
}

private struct AppKitSecureField: NSViewRepresentable {
    let placeholder: String
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeNSView(context: Context) -> NSSecureTextField {
        let textField = NSSecureTextField(string: text)
        configure(textField, coordinator: context.coordinator)
        return textField
    }

    func updateNSView(_ nsView: NSSecureTextField, context: Context) {
        if nsView.stringValue != text {
            nsView.stringValue = text
        }
        nsView.placeholderString = placeholder
    }

    private func configure(_ textField: NSSecureTextField, coordinator: Coordinator) {
        textField.isEditable = true
        textField.isSelectable = true
        textField.isBordered = true
        textField.isBezeled = true
        textField.bezelStyle = .roundedBezel
        textField.drawsBackground = true
        textField.focusRingType = .default
        textField.placeholderString = placeholder
        textField.delegate = coordinator
        textField.maximumNumberOfLines = 1
        textField.lineBreakMode = .byTruncatingTail
        textField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        @Binding private var text: String

        init(text: Binding<String>) {
            _text = text
        }

        func controlTextDidChange(_ obj: Notification) {
            guard let textField = obj.object as? NSTextField else { return }
            if text != textField.stringValue {
                text = textField.stringValue
            }
        }
    }
}
