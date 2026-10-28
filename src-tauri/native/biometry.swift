import AppKit
import CryptoKit
import Foundation
import LocalAuthentication
import LocalAuthenticationEmbeddedUI
import Security

// This file never creates a Keychain item. CryptoKit returns an encrypted, device-bound
// Secure Enclave key representation; Rust owns its bounded, private local file.
private let payloadLength = 80
private let maximumBlobLength = 32 * 1024
private let maximumEnclaveLength = 16 * 1024
private let magic = Data("ENOTE-SE-WRAP-V1".utf8)
private let domain = Data("com.encryptednote.app.quick-unlock.p256.v1".utf8)
private let reason = "unlock your encrypted-note Vault"
private let authenticationTimeout: DispatchTimeInterval = .seconds(120)

private enum BiometryFailure: Error { case cancelled, unavailable, malformed, failed }

private func freshContext() -> LAContext {
    let context = LAContext()
    context.localizedReason = reason
    context.localizedFallbackTitle = ""
    context.touchIDAuthenticationAllowableReuseDuration = 0
    return context
}

// The one authentication context of a protected read, paired with its embedded view. It is
// claimed once; a read without a paired view never starts, so no system alert can appear.
private final class Paired {
    let context: LAContext
    var claimed = false
    init(_ context: LAContext) { self.context = context }
}

// Cancellation can arrive before registration, during a protected operation, or between
// enrollment and its read-back. Rust releases the id only when the last request handle drops.
private final class Requests: @unchecked Sendable {
    static let shared = Requests()
    private let lock = NSLock()
    private var contexts: [UInt64: LAContext] = [:]
    private var paired: [UInt64: Paired] = [:]
    private var cancelled: Set<UInt64> = []

    func begin(_ id: UInt64) throws -> LAContext {
        lock.lock()
        defer { lock.unlock() }
        guard id != 0, !cancelled.contains(id), contexts[id] == nil else {
            throw BiometryFailure.cancelled
        }
        let context = freshContext()
        contexts[id] = context
        return context
    }

    func end(_ id: UInt64, _ context: LAContext) {
        context.invalidate()
        lock.lock()
        defer { lock.unlock() }
        if contexts[id] === context { contexts.removeValue(forKey: id) }
    }

    // Main thread only (the caller checks). One view per request, created with the exact
    // context the later Secure Enclave operation uses.
    func pair(_ id: UInt64) -> NSView? {
        lock.lock()
        defer { lock.unlock() }
        guard id != 0, !cancelled.contains(id), paired[id] == nil else { return nil }
        let context = freshContext()
        let view = LAAuthenticationView(context: context, controlSize: .large)
        paired[id] = Paired(context)
        return view
    }

    func claim(_ id: UInt64) throws -> LAContext {
        lock.lock()
        defer { lock.unlock() }
        // No paired view means no embedded presentation: refuse rather than fall back to the
        // system alert, which takes focus away from the protected window.
        guard !cancelled.contains(id), let entry = paired[id], !entry.claimed else {
            throw BiometryFailure.cancelled
        }
        entry.claimed = true
        return entry.context
    }

    func cancel(_ id: UInt64) {
        lock.lock()
        cancelled.insert(id)
        let context = contexts[id]
        let embedded = paired[id]?.context
        lock.unlock()
        context?.invalidate()
        embedded?.invalidate()
    }

    func release(_ id: UInt64) {
        lock.lock()
        let context = contexts.removeValue(forKey: id)
        let embedded = paired.removeValue(forKey: id)?.context
        cancelled.remove(id)
        lock.unlock()
        context?.invalidate()
        embedded?.invalidate()
    }

    func check(_ id: UInt64) throws {
        lock.lock()
        defer { lock.unlock() }
        if cancelled.contains(id) { throw BiometryFailure.cancelled }
    }
}

@_cdecl("enote_biometry_cancel")
func enoteBiometryCancel(_ id: UInt64) { Requests.shared.cancel(id) }

@_cdecl("enote_biometry_release")
func enoteBiometryRelease(_ id: UInt64) { Requests.shared.release(id) }

// Returns a +1 retained LAAuthenticationView for this request, or nil. Main thread only;
// Rust installs it in the protected window before the read starts and releases it after.
@_cdecl("enote_biometry_view")
func enoteBiometryView(_ id: UInt64) -> UnsafeMutableRawPointer? {
    // A Thread check, not MainActor.assumeIsolated: that would link the Swift Concurrency
    // runtime, which this static bridge does not ship or load.
    guard Thread.isMainThread, let view = Requests.shared.pair(id) else { return nil }
    return Unmanaged.passRetained(view).toOpaque()
}

@_cdecl("enote_biometry_available")
func enoteBiometryAvailable() -> Int32 {
    guard SecureEnclave.isAvailable else { return 0 }
    let context = LAContext()
    defer { context.invalidate() }
    return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil) ? 1 : 0
}

private func status(_ error: Error) -> Int32 {
    if let error = error as? BiometryFailure {
        switch error {
        case .cancelled: return 1
        case .unavailable: return 2
        case .malformed: return 3
        case .failed: return 4
        }
    }
    if let error = error as? LAError,
       [.userCancel, .appCancel, .systemCancel].contains(error.code) { return 1 }
    return 4
}

private final class Outcome: @unchecked Sendable { var error: Error? = BiometryFailure.failed }

// Blocks the calling worker thread (never the main thread, which draws the paired view).
private func authenticate(_ context: LAContext, _ access: SecAccessControl) throws {
    let done = DispatchSemaphore(value: 0)
    let outcome = Outcome()
    context.evaluateAccessControl(access, operation: .useKeyKeyExchange,
        localizedReason: reason) { success, error in
        outcome.error = success ? nil : (error ?? BiometryFailure.failed)
        done.signal()
    }
    guard done.wait(timeout: .now() + authenticationTimeout) == .success else {
        context.invalidate()
        throw BiometryFailure.cancelled
    }
    if let error = outcome.error { throw error }
}

private func randomBytes(_ count: Int) throws -> Data {
    var result = Data(count: count)
    let outcome = result.withUnsafeMutableBytes {
        SecRandomCopyBytes(kSecRandomDefault, count, $0.baseAddress!)
    }
    guard outcome == errSecSuccess else { throw BiometryFailure.failed }
    return result
}

private struct Envelope {
    let header: Data
    let enclave: Data
    let enclavePublic: Data
    let ephemeralPublic: Data
    let salt: Data
    let binding: Data
    let nonce: Data
    let ciphertext: Data
    let tag: Data

    init(_ data: Data) throws {
        guard data.count <= maximumBlobLength, data.count >= 323,
              data.prefix(magic.count) == magic else { throw BiometryFailure.malformed }
        var cursor = magic.count
        func take(_ count: Int) throws -> Data {
            guard count >= 0, count <= data.count - cursor else { throw BiometryFailure.malformed }
            defer { cursor += count }
            return data.subdata(in: cursor ..< cursor + count)
        }
        let size = try take(4).reduce(0) { ($0 << 8) | Int($1) }
        guard size > 0, size <= maximumEnclaveLength else { throw BiometryFailure.malformed }
        enclave = try take(size)
        enclavePublic = try take(65)
        ephemeralPublic = try take(65)
        guard enclavePublic.first == 4, ephemeralPublic.first == 4 else { throw BiometryFailure.malformed }
        salt = try take(32)
        binding = try take(32)
        nonce = try take(12)
        header = data.prefix(cursor)
        ciphertext = try take(payloadLength)
        tag = try take(16)
        guard cursor == data.count else { throw BiometryFailure.malformed }
    }
}

// The complete framing and both public keys are authenticated and included in HKDF info.
private func wrappingKey(_ secret: SharedSecret, salt: Data, header: Data) -> SymmetricKey {
    secret.hkdfDerivedSymmetricKey(using: SHA256.self, salt: salt,
        sharedInfo: domain + header, outputByteCount: 32)
}

@_cdecl("enote_biometry_wrap")
func enoteBiometryWrap(_ id: UInt64, _ input: UnsafePointer<UInt8>?, _ length: Int,
    _ output: UnsafeMutablePointer<UInt8>?, _ capacity: Int,
    _ written: UnsafeMutablePointer<Int>?) -> Int32 {
    written?.pointee = 0
    guard let input, let output, let written, length == payloadLength,
          capacity >= maximumBlobLength else { return 3 }
    do {
        guard SecureEnclave.isAvailable else { throw BiometryFailure.unavailable }
        let context = try Requests.shared.begin(id)
        defer { Requests.shared.end(id, context) }
        guard let access = SecAccessControlCreateWithFlags(nil,
            kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], nil) else { throw BiometryFailure.failed }
        let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(
            compactRepresentable: false, accessControl: access, authenticationContext: context)
        let enclaveBlob = enclave.dataRepresentation
        guard enclaveBlob.count > 0, enclaveBlob.count <= maximumEnclaveLength else {
            throw BiometryFailure.malformed
        }
        let ephemeral = P256.KeyAgreement.PrivateKey(compactRepresentable: false)
        let salt = try randomBytes(32)
        let nonce = AES.GCM.Nonce()
        var header = magic
        let size = UInt32(enclaveBlob.count)
        header.append(contentsOf: [UInt8(size >> 24), UInt8((size >> 16) & 255),
            UInt8((size >> 8) & 255), UInt8(size & 255)])
        header += enclaveBlob
        header += enclave.publicKey.x963Representation
        header += ephemeral.publicKey.x963Representation
        header += salt
        header += Data(bytes: input.advanced(by: 16), count: 32)
        nonce.withUnsafeBytes { header.append(contentsOf: $0) }
        // Encryption uses the public enclave key; only the later private ECDH operation
        // can reconstruct this secret. The software ephemeral private key is not saved.
        let shared = try ephemeral.sharedSecretFromKeyAgreement(with: enclave.publicKey)
        let key = wrappingKey(shared, salt: salt, header: header)
        let borrowed = Data(bytesNoCopy: UnsafeMutableRawPointer(mutating: input),
            count: payloadLength, deallocator: .none)
        let sealed = try AES.GCM.seal(borrowed, using: key, nonce: nonce, authenticating: header)
        let envelope = header + sealed.ciphertext + sealed.tag
        guard envelope.count <= maximumBlobLength else { throw BiometryFailure.malformed }
        try Requests.shared.check(id)
        envelope.copyBytes(to: output, count: envelope.count)
        written.pointee = envelope.count
        return 0
    } catch { return status(error) }
}

@_cdecl("enote_biometry_open")
func enoteBiometryOpen(_ id: UInt64, _ input: UnsafePointer<UInt8>?, _ length: Int,
    _ output: UnsafeMutablePointer<UInt8>?, _ capacity: Int) -> Int32 {
    guard let input, let output, length > 0, length <= maximumBlobLength,
          capacity == payloadLength else { return 3 }
    do {
        let envelope = try Envelope(Data(bytes: input, count: length))
        let context = try Requests.shared.claim(id)
        defer { context.invalidate() }
        guard let access = SecAccessControlCreateWithFlags(nil,
            kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], nil) else { throw BiometryFailure.failed }
        // Biometrics only, presented by the paired view inside the protected window. Success
        // here authorizes nothing by itself: only the enclave operation below releases data.
        try authenticate(context, access)
        try Requests.shared.check(id)
        // Any further authentication UI would appear outside the protected window: refuse it.
        context.interactionNotAllowed = true
        let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(
            dataRepresentation: envelope.enclave, authenticationContext: context)
        guard enclave.publicKey.x963Representation == envelope.enclavePublic else {
            throw BiometryFailure.malformed
        }
        let ephemeral = try P256.KeyAgreement.PublicKey(x963Representation: envelope.ephemeralPublic)
        // This actual private-key operation is controlled by the stored enclave ACL.
        // No boolean authentication result ever authorizes plaintext release.
        let shared = try enclave.sharedSecretFromKeyAgreement(with: ephemeral)
        let key = wrappingKey(shared, salt: envelope.salt, header: envelope.header)
        let nonce = try AES.GCM.Nonce(data: envelope.nonce)
        let sealed = try AES.GCM.SealedBox(nonce: nonce, ciphertext: envelope.ciphertext, tag: envelope.tag)
        var plaintext = try AES.GCM.open(sealed, using: key, authenticating: envelope.header)
        defer { plaintext.resetBytes(in: 0 ..< plaintext.count) }
        guard plaintext.count == payloadLength,
              plaintext.subdata(in: 16 ..< 48) == envelope.binding else { throw BiometryFailure.malformed }
        try Requests.shared.check(id)
        plaintext.copyBytes(to: output, count: payloadLength)
        return 0
    } catch { return status(error) }
}

@_cdecl("enote_biometry_validate")
func enoteBiometryValidate(_ input: UnsafePointer<UInt8>?, _ length: Int) -> Int32 {
    guard let input, length > 0, length <= maximumBlobLength else { return 3 }
    do { _ = try Envelope(Data(bytes: input, count: length)); return 0 }
    catch { return 3 }
}
