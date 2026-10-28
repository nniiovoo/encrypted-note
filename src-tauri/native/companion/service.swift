import CryptoKit
import Darwin
import Foundation

// The Agent Companion helper (ADR-0009): an XPC service inside encrypted-note.app with its own
// App Sandbox container. It cannot open the Vault's container, and the Vault process cannot open
// its container. It supervises the pinned Node runtime that runs the companion's owner broker.
//
// - Only the containing app may connect. When the app goes away, the broker is stopped and this
//   helper exits; it never restarts the broker by itself.
// - The broker is launched from fixed bundle paths with a cleared environment, no inherited
//   descriptors except its own pipes, and stderr discarded.
// - Broker output is parsed as a bounded protocol of fixed fields. Nothing it prints, and nothing
//   from the plan, the Agent Key, the agent token or OpenAI, is ever forwarded or logged.

enum State: Int32 { case stopped = 1, starting = 2, running = 3, stopping = 4, expired = 5, error = 6 }

enum Code: Int32 {
    case none = 0
    case planMissing = 1, planInvalid = 2, wrongIdentity = 3, codeUnprotected = 4, agentUnverified = 5
    case credentialMissing = 6, credentialInvalid = 7, handoffUnavailable = 8, portUnavailable = 9
    case grantInactive = 10, controlInvalid = 11, internalError = 12
    case runtimeMissing = 20, policyMismatch = 21, launchFailed = 22, startTimeout = 23
    case unexpectedExit = 24, protocolInvalid = 25, busy = 26, installInvalid = 27
}

private let brokerCodes: [String: Code] = [
    "plan_missing": .planMissing, "plan_invalid": .planInvalid, "wrong_identity": .wrongIdentity,
    "code_unprotected": .codeUnprotected, "agent_unverified": .agentUnverified,
    "credential_missing": .credentialMissing, "credential_invalid": .credentialInvalid,
    "handoff_unavailable": .handoffUnavailable, "port_unavailable": .portUnavailable,
    "grant_inactive": .grantInactive, "control_invalid": .controlInvalid, "internal": .internalError,
]
private let stopReasons: Set<String> = ["owner", "expired", "parent", "error"]

// Readiness flags (bit numbers mirrored in src-tauri/src/companion.rs).
private let runtimeFlag: UInt32 = 1 << 0
private let checkFlags: [(String, UInt32)] = [
    ("plan", 1 << 1), ("credential", 1 << 2), ("handoff", 1 << 3), ("code", 1 << 4), ("agent", 1 << 5),
]

private let maxLineBytes = 256
private let maxPendingBytes = 4096
private let maxPlanBytes = 8192
private let maxCredentialBytes = 2048
private let minCredentialBytes = 37 // "OPENAI_AGENT_API_KEY=" plus the shortest accepted key
private let maxGrantSeconds: Int64 = 3600

private struct Paths {
    let node: String
    let entry: String
    let policy: String
    let data: String

    init() {
        let bundle = Bundle.main.bundlePath
        let companion = bundle + "/Contents/Resources/companion"
        node = bundle + "/Contents/MacOS/node"
        entry = companion + "/isolation/integrated-broker.mjs"
        policy = companion + "/policy.json"
        // This helper's own container; never the Vault's.
        data = NSHomeDirectory() + "/Library/Application Support/encrypted-note-companion"
    }
}

/// A child started with posix_spawn: exactly stdin (control), stdout (protocol) and stderr
/// (/dev/null). POSIX_SPAWN_CLOEXEC_DEFAULT closes every other descriptor.
private final class Child {
    let pid: pid_t
    var control: Int32 // write end of the child's stdin, -1 once closed
    let output: Int32 // read end of the child's stdout
    /// Signalled when every byte the child printed has been read, so an exit is never handled
    /// before the child's last protocol line.
    let drained = DispatchSemaphore(value: 0)

    private init(pid: pid_t, control: Int32, output: Int32) {
        self.pid = pid
        self.control = control
        self.output = output
    }

    static func spawn(paths: Paths, arguments: [String]) -> Child? {
        var input: [Int32] = [-1, -1]
        var stdout: [Int32] = [-1, -1]
        guard pipe(&input) == 0 else { return nil }
        guard pipe(&stdout) == 0 else { close(input[0]); close(input[1]); return nil }
        for fd in [input[1], stdout[0]] { _ = fcntl(fd, F_SETFD, FD_CLOEXEC) }

        var actions: posix_spawn_file_actions_t? = nil
        var attributes: posix_spawnattr_t? = nil
        posix_spawn_file_actions_init(&actions)
        posix_spawnattr_init(&attributes)
        defer {
            posix_spawn_file_actions_destroy(&actions)
            posix_spawnattr_destroy(&attributes)
        }
        posix_spawn_file_actions_adddup2(&actions, input[0], STDIN_FILENO)
        posix_spawn_file_actions_adddup2(&actions, stdout[1], STDOUT_FILENO)
        posix_spawn_file_actions_addopen(&actions, STDERR_FILENO, "/dev/null", O_WRONLY, 0)
        var defaults = sigset_t()
        sigfillset(&defaults)
        posix_spawnattr_setsigdefault(&attributes, &defaults)
        var empty = sigset_t()
        sigemptyset(&empty)
        posix_spawnattr_setsigmask(&attributes, &empty)
        posix_spawnattr_setflags(&attributes,
            Int16(POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK))

        let argv = [paths.node, paths.entry] + arguments
        // Only HOME (this helper's container) and a fixed PATH: nothing inherited.
        let environment = ["HOME=" + NSHomeDirectory(), "PATH=/usr/bin:/bin", "LANG=C"]
        var cArgv: [UnsafeMutablePointer<CChar>?] = argv.map { strdup($0) } + [nil]
        var cEnv: [UnsafeMutablePointer<CChar>?] = environment.map { strdup($0) } + [nil]
        defer {
            cArgv.forEach { free($0) }
            cEnv.forEach { free($0) }
        }
        var pid: pid_t = 0
        let spawned = paths.data.withCString { data -> Int32 in
            // The child's working directory is this helper's data folder.
            posix_spawn_file_actions_addchdir_np(&actions, data)
            return posix_spawn(&pid, paths.node, &actions, &attributes, &cArgv, &cEnv)
        }
        close(input[0])
        close(stdout[1])
        guard spawned == 0 else {
            close(input[1])
            close(stdout[0])
            return nil
        }
        return Child(pid: pid, control: input[1], output: stdout[0])
    }

    func closeControl() {
        if control >= 0 { close(control); control = -1 }
    }
}

final class Broker: NSObject, EnoteCompanionService {
    private let paths = Paths()
    private let queue = DispatchQueue(label: "com.encryptednote.companion.operations")
    private let lock = NSLock()
    // Guarded by `lock`.
    private var state = State.stopped
    private var flags: UInt32 = 0
    private var expires: Int64 = 0
    private var code = Code.none
    private var child: Child?
    private var stopReason: String?
    private var readySignal: DispatchSemaphore?
    private var exitSignal: DispatchSemaphore?
    private var checkedAt: Date?

    // ---------- XPC operations: each answers with four numbers ----------

    func status(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void) {
        queue.async {
            if !self.isRunning, (self.checkedAt.map { Date().timeIntervalSince($0) > 15 } ?? true) {
                self.refreshReadiness()
            }
            self.answer(reply)
        }
    }

    func start(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void) {
        queue.async {
            self.startBroker()
            self.answer(reply)
        }
    }

    func stop(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void) {
        queue.async {
            self.stopBroker()
            self.answer(reply)
        }
    }

    func install(plan: FileHandle, credential: FileHandle,
                 withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void) {
        let planFd = dup(plan.fileDescriptor)
        let credentialFd = dup(credential.fileDescriptor)
        queue.async {
            defer {
                if planFd >= 0 { close(planFd) }
                if credentialFd >= 0 { close(credentialFd) }
            }
            if self.isRunning {
                self.set(code: .busy)
            } else if planFd < 0 || credentialFd < 0 || !self.installFiles(planFd: planFd, credentialFd: credentialFd) {
                self.set(state: .error, code: .installInvalid)
            } else {
                self.set(state: .stopped, code: .none)
            }
            self.refreshReadiness()
            self.answer(reply)
        }
    }

    /// The containing app went away: stop the broker, then end this helper.
    func shutdownForExit() {
        queue.async {
            self.stopBroker()
            exit(0)
        }
    }

    // ---------- state ----------

    private var isRunning: Bool {
        lock.lock()
        defer { lock.unlock() }
        return child != nil
    }

    private func answer(_ reply: (Int32, UInt32, Int64, Int32) -> Void) {
        lock.lock()
        let values = (state.rawValue, flags, expires, code.rawValue)
        lock.unlock()
        reply(values.0, values.1, values.2, values.3)
    }

    private func set(state newState: State? = nil, code newCode: Code) {
        lock.lock()
        if let newState { state = newState }
        code = newCode
        lock.unlock()
    }

    // ---------- runtime and readiness ----------

    /// The pinned runtime is present and the bundled policy is the one this build was made for.
    private func verifyRuntime() -> Code {
        let files = FileManager.default
        guard files.isExecutableFile(atPath: paths.node), files.fileExists(atPath: paths.entry),
              let expected = Bundle.main.object(forInfoDictionaryKey: "EnoteCompanionPolicySHA256") as? String,
              expected.count == 64, let policy = files.contents(atPath: paths.policy),
              policy.count <= maxPlanBytes else { return .runtimeMissing }
        let digest = SHA256.hash(data: policy).map { String(format: "%02x", $0) }.joined()
        return digest == expected ? .none : .policyMismatch
    }

    private func ensureDataFolder() -> Bool {
        do {
            try FileManager.default.createDirectory(atPath: paths.data, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
            return chmod(paths.data, 0o700) == 0
        } catch { return false }
    }

    /// Readiness from the broker's metadata-only check mode, plus the runtime check here.
    private func refreshReadiness() {
        checkedAt = Date()
        var next: UInt32 = 0
        guard verifyRuntime() == .none, ensureDataFolder() else {
            lock.lock(); flags = 0; lock.unlock()
            return
        }
        next |= runtimeFlag
        if let line = runCheck(), let event = parse(line), event["event"] as? String == "check",
           Set(event.keys) == Set(["v", "event"] + checkFlags.map { $0.0 }) {
            for (key, bit) in checkFlags where (event[key] as? NSNumber).map({ isBool($0) && $0.boolValue }) == true {
                next |= bit
            }
        }
        lock.lock(); flags = next; lock.unlock()
    }

    /// Runs `integrated-broker.mjs --check` and returns its single protocol line.
    private func runCheck() -> String? {
        guard let checker = Child.spawn(paths: paths, arguments: ["--check"]) else { return nil }
        checker.closeControl()
        let deadline = Date().addingTimeInterval(10)
        var bytes = [UInt8]()
        var buffer = [UInt8](repeating: 0, count: 512)
        _ = fcntl(checker.output, F_SETFL, O_NONBLOCK)
        while Date() < deadline, bytes.count <= maxLineBytes {
            let count = read(checker.output, &buffer, buffer.count)
            if count > 0 { bytes.append(contentsOf: buffer[0..<count]); continue }
            if count == 0 { break }
            if errno != EAGAIN && errno != EINTR { break }
            usleep(20_000)
        }
        close(checker.output)
        kill(checker.pid, SIGKILL)
        var status: Int32 = 0
        waitpid(checker.pid, &status, 0)
        guard bytes.count <= maxLineBytes, bytes.last == 10, bytes.dropLast().allSatisfy({ $0 != 10 }) else { return nil }
        return String(bytes: bytes.dropLast(), encoding: .utf8)
    }

    // ---------- protocol ----------

    private func isBool(_ number: NSNumber) -> Bool { CFGetTypeID(number) == CFBooleanGetTypeID() }

    /// One bounded protocol line: a JSON object with version 1 and only fixed fields.
    private func parse(_ line: String) -> [String: Any]? {
        guard line.utf8.count <= maxLineBytes,
              let object = try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any],
              (object["v"] as? NSNumber).map({ !isBool($0) && $0.int64Value == 1 }) == true,
              object["event"] is String else { return nil }
        return object
    }

    private func handle(line: String) {
        guard let event = parse(line), let kind = event["event"] as? String else { return violation() }
        let keys = Set(event.keys)
        switch kind {
        case "ready" where keys == ["v", "event", "expiresAt"]:
            let now = Int64(Date().timeIntervalSince1970)
            guard let number = event["expiresAt"] as? NSNumber, !isBool(number),
                  number.int64Value > now, number.int64Value <= now + maxGrantSeconds + 5 else { return violation() }
            lock.lock()
            state = .running
            expires = number.int64Value
            code = .none
            let signal = readySignal
            lock.unlock()
            signal?.signal()
        case "stopped" where keys == ["v", "event", "reason"]:
            guard let reason = event["reason"] as? String, stopReasons.contains(reason) else { return violation() }
            lock.lock(); stopReason = reason; lock.unlock()
        case "error" where keys == ["v", "event", "code"]:
            guard let name = event["code"] as? String, let mapped = brokerCodes[name] else { return violation() }
            lock.lock()
            code = mapped
            state = .error
            let signal = readySignal
            lock.unlock()
            signal?.signal()
        default:
            violation()
        }
    }

    /// Anything outside the protocol ends the broker; its output is never kept.
    private func violation() {
        lock.lock()
        code = .protocolInvalid
        state = .error
        let running = child
        let signal = readySignal
        lock.unlock()
        if let running { kill(running.pid, SIGKILL) }
        signal?.signal()
    }

    private func readOutput(_ running: Child) {
        DispatchQueue.global(qos: .utility).async {
            var pending = [UInt8]()
            var buffer = [UInt8](repeating: 0, count: 1024)
            while true {
                let count = read(running.output, &buffer, buffer.count)
                if count < 0 && errno == EINTR { continue }
                if count <= 0 { break }
                pending.append(contentsOf: buffer[0..<count])
                while let newline = pending.firstIndex(of: 10) {
                    let line = Array(pending[..<newline])
                    pending.removeSubrange(...newline)
                    guard line.count <= maxLineBytes, let text = String(bytes: line, encoding: .utf8) else {
                        self.violation(); continue
                    }
                    self.handle(line: text)
                }
                if pending.count > maxPendingBytes { self.violation(); pending.removeAll() }
            }
            close(running.output)
            running.drained.signal()
        }
    }

    /// A dedicated thread reaps the child, so an exit before any watcher is armed is never missed.
    private func watchExit(_ running: Child) {
        Thread.detachNewThread { [weak self] in
            var status: Int32 = 0
            while waitpid(running.pid, &status, 0) < 0 && errno == EINTR {}
            _ = running.drained.wait(timeout: .now() + 2)
            self?.exited(running)
        }
    }

    private func exited(_ running: Child) {
        lock.lock()
        guard child === running else { lock.unlock(); return }
        child = nil
        running.closeControl()
        switch stopReason {
        case _ where state == .stopping: state = .stopped // the owner's stop, however it ended
        case "expired": state = .expired
        case "owner", "parent": if state != .error { state = .stopped }
        case "error": state = .error; if code == .none { code = .controlInvalid }
        default:
            if state != .error { state = .error; code = .unexpectedExit }
        }
        let signals = [readySignal, exitSignal]
        lock.unlock()
        signals.forEach { $0?.signal() }
    }

    // ---------- start, stop, install ----------

    private func startBroker() {
        lock.lock()
        let busy = child != nil
        lock.unlock()
        if busy { return }
        let runtime = verifyRuntime()
        guard runtime == .none else { return set(state: .error, code: runtime) }
        guard ensureDataFolder() else { return set(state: .error, code: .launchFailed) }
        let ready = DispatchSemaphore(value: 0)
        let gone = DispatchSemaphore(value: 0)
        lock.lock()
        state = .starting
        code = .none
        expires = 0
        stopReason = nil
        readySignal = ready
        exitSignal = gone
        lock.unlock()
        guard let running = Child.spawn(paths: paths, arguments: []) else {
            return set(state: .error, code: .launchFailed)
        }
        lock.lock(); child = running; lock.unlock()
        readOutput(running)
        watchExit(running)
        // Code-tree verification and the credential read happen before "ready".
        if ready.wait(timeout: .now() + 30) == .timedOut {
            kill(running.pid, SIGKILL)
            _ = gone.wait(timeout: .now() + 2)
            set(state: .error, code: .startTimeout)
        }
    }

    private func stopBroker() {
        lock.lock()
        guard let running = child else { lock.unlock(); return }
        if state != .error { state = .stopping }
        let gone = exitSignal
        lock.unlock()
        if running.control >= 0 {
            let line = Array("{\"v\":1,\"op\":\"stop\"}\n".utf8)
            _ = line.withUnsafeBytes { write(running.control, $0.baseAddress, $0.count) }
        }
        running.closeControl() // EOF also stops the broker
        if gone?.wait(timeout: .now() + 5) == .timedOut {
            kill(running.pid, SIGTERM)
            if gone?.wait(timeout: .now() + 2) == .timedOut {
                kill(running.pid, SIGKILL)
                _ = gone?.wait(timeout: .now() + 2)
            }
        }
        lock.lock()
        if state == .stopping { state = .stopped }
        lock.unlock()
    }

    /// Copies the two owner-chosen files into this helper's own container, owner-only.
    private func installFiles(planFd: Int32, credentialFd: Int32) -> Bool {
        guard ensureDataFolder() else { return false }
        var plan = readBounded(planFd, limit: maxPlanBytes)
        var credential = readBounded(credentialFd, limit: maxCredentialBytes)
        defer {
            plan.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) }
            credential.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) }
        }
        guard !plan.isEmpty, plan.count <= maxPlanBytes,
              credential.count >= minCredentialBytes, credential.count <= maxCredentialBytes else { return false }
        return replace(paths.data + "/plan.json", with: plan)
            && replace(paths.data + "/credential.env", with: credential)
    }

    private func readBounded(_ fd: Int32, limit: Int) -> [UInt8] {
        var bytes = [UInt8]()
        var buffer = [UInt8](repeating: 0, count: 1024)
        defer { buffer.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) } }
        _ = lseek(fd, 0, SEEK_SET)
        while bytes.count <= limit {
            let count = read(fd, &buffer, buffer.count)
            if count < 0 && errno == EINTR { continue }
            if count <= 0 { break }
            bytes.append(contentsOf: buffer[0..<count])
        }
        return bytes
    }

    private func replace(_ path: String, with bytes: [UInt8]) -> Bool {
        let temporary = path + ".new"
        unlink(temporary)
        let fd = open(temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard fd >= 0 else { return false }
        var written = 0
        let ok = bytes.withUnsafeBytes { raw -> Bool in
            while written < raw.count {
                let count = write(fd, raw.baseAddress! + written, raw.count - written)
                if count < 0 && errno == EINTR { continue }
                if count <= 0 { return false }
                written += count
            }
            return fsync(fd) == 0
        }
        close(fd)
        guard ok, fchmodat(AT_FDCWD, temporary, 0o600, AT_SYMLINK_NOFOLLOW) == 0 || chmod(temporary, 0o600) == 0,
              rename(temporary, path) == 0 else {
            unlink(temporary)
            return false
        }
        return true
    }
}

final class Listener: NSObject, NSXPCListenerDelegate {
    let broker = Broker()

    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        // Only the app this helper is bundled in (its identifier without ".companion").
        guard let own = Bundle.main.bundleIdentifier, own.hasSuffix(".companion") else { return false }
        let app = String(own.dropLast(".companion".count))
        guard app.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "." || $0 == "-" }) else { return false }
        connection.setCodeSigningRequirement("identifier \"\(app)\"")
        connection.exportedInterface = NSXPCInterface(with: EnoteCompanionService.self)
        connection.exportedObject = broker
        connection.invalidationHandler = { [broker] in broker.shutdownForExit() }
        connection.interruptionHandler = { [broker] in broker.shutdownForExit() }
        connection.resume()
        return true
    }
}

@main
enum CompanionHelper {
    static func main() {
        // A broker that exits mid-write must not take this helper down with SIGPIPE.
        signal(SIGPIPE, SIG_IGN)
        let delegate = Listener()
        let listener = NSXPCListener.service()
        listener.delegate = delegate
        listener.resume() // services never return from here
    }
}
