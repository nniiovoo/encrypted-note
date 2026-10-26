import Foundation

// App side of the Agent Companion (ADR-0009). It only forwards four fixed operations to the
// bundled helper and copies four numbers back. It never reads files, keys or helper output.
// Return codes: 0 reply received, 1 helper not bundled, 2 connection failed, 3 timed out,
// 4 invalid call.

private final class Connection {
    static let shared = Connection()
    private let lock = NSLock()
    private var connection: NSXPCConnection?

    /// The helper is `<app identifier>.companion`, inside this app's Contents/XPCServices.
    var serviceName: String? {
        guard let id = Bundle.main.bundleIdentifier, !id.isEmpty else { return nil }
        return id + ".companion"
    }

    var bundled: Bool {
        guard let name = serviceName else { return false }
        let url = Bundle.main.bundleURL.appendingPathComponent("Contents/XPCServices/\(name).xpc")
        var isDirectory: ObjCBool = false
        return FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) && isDirectory.boolValue
    }

    func proxy(onError: @escaping () -> Void) -> EnoteCompanionService? {
        lock.lock()
        defer { lock.unlock() }
        if connection == nil, let name = serviceName {
            let fresh = NSXPCConnection(serviceName: name)
            fresh.remoteObjectInterface = NSXPCInterface(with: EnoteCompanionService.self)
            // Invalidated: drop it; the next call opens a new connection. Interrupted (the helper
            // exited): the helper starts again only for the next call, with nothing running.
            fresh.invalidationHandler = { [weak self] in self?.drop() }
            fresh.resume()
            connection = fresh
        }
        return connection?.remoteObjectProxyWithErrorHandler { _ in onError() } as? EnoteCompanionService
    }

    private func drop() {
        lock.lock()
        connection = nil
        lock.unlock()
    }
}

private final class Reply {
    private let lock = NSLock()
    private var done = false
    private var values: (Int32, UInt32, Int64, Int32) = (0, 0, 0, 0)
    private var failed = false
    let signal = DispatchSemaphore(value: 0)

    func set(_ state: Int32, _ flags: UInt32, _ expires: Int64, _ error: Int32) {
        lock.lock()
        defer { lock.unlock() }
        guard !done else { return }
        done = true
        values = (state, flags, expires, error)
        signal.signal()
    }

    func fail() {
        lock.lock()
        defer { lock.unlock() }
        guard !done else { return }
        done = true
        failed = true
        signal.signal()
    }

    func copy(_ state: UnsafeMutablePointer<Int32>, _ flags: UnsafeMutablePointer<UInt32>,
              _ expires: UnsafeMutablePointer<Int64>, _ error: UnsafeMutablePointer<Int32>) -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        if failed { return 2 }
        (state.pointee, flags.pointee, expires.pointee, error.pointee) = values
        return 0
    }
}

@_cdecl("enote_companion_bundled")
func enoteCompanionBundled() -> Int32 { Connection.shared.bundled ? 1 : 0 }

@_cdecl("enote_companion_call")
func enoteCompanionCall(_ operation: Int32, _ planFd: Int32, _ credentialFd: Int32, _ timeoutMs: Int32,
    _ state: UnsafeMutablePointer<Int32>?, _ flags: UnsafeMutablePointer<UInt32>?,
    _ expires: UnsafeMutablePointer<Int64>?, _ error: UnsafeMutablePointer<Int32>?) -> Int32 {
    guard let state, let flags, let expires, let error, timeoutMs > 0, timeoutMs <= 60_000 else { return 4 }
    guard Connection.shared.bundled else { return 1 }
    let reply = Reply()
    guard let proxy = Connection.shared.proxy(onError: { reply.fail() }) else { return 2 }
    let receive: (Int32, UInt32, Int64, Int32) -> Void = { reply.set($0, $1, $2, $3) }
    switch operation {
    case 1: proxy.status(withReply: receive)
    case 2: proxy.start(withReply: receive)
    case 3: proxy.stop(withReply: receive)
    case 4:
        guard planFd >= 0, credentialFd >= 0 else { return 4 }
        // XPC duplicates the descriptors; Rust keeps and closes its own.
        proxy.install(plan: FileHandle(fileDescriptor: planFd, closeOnDealloc: false),
                      credential: FileHandle(fileDescriptor: credentialFd, closeOnDealloc: false),
                      withReply: receive)
    default: return 4
    }
    if reply.signal.wait(timeout: .now() + .milliseconds(Int(timeoutMs))) == .timedOut { return 3 }
    return reply.copy(state, flags, expires, error)
}
