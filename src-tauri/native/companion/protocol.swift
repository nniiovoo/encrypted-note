import Foundation

// The Agent Companion's XPC contract, compiled into both the app's client and the helper.
// Every reply is four fixed numbers: state, readiness flags, grant expiry (Unix seconds) and an
// error code. No text, path, key, token or helper output can cross this boundary (ADR-0009).
@objc(EnoteCompanionService) protocol EnoteCompanionService {
    func status(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void)
    func start(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void)
    func stop(withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void)
    // Owner setup: two already-open files chosen in OS panels. The app never reads them.
    func install(plan: FileHandle, credential: FileHandle,
                 withReply reply: @escaping (Int32, UInt32, Int64, Int32) -> Void)
}
