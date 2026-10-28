import Foundation

// Fictional-data verification harness (scripts/verify-companion.sh). A stand-in for the Vault app:
// the Vault app's entitlements, the same companion client code, a test identifier and a FAKE
// canary where the Vault folder would be. It prints only the four numbers the client returns.
// Steps run in one process, because the helper stops everything when this process goes away:
//   canary | status | start | stop | install:PLAN:CREDENTIAL | sleep:SECONDS | pid
let vault = NSHomeDirectory() + "/Library/Application Support/" + (Bundle.main.bundleIdentifier ?? "") + "/Vault.noindex"

func call(_ name: String, _ operation: Int32, _ planFd: Int32 = -1, _ credentialFd: Int32 = -1) {
    var state: Int32 = 0, flags: UInt32 = 0, expires: Int64 = 0, error: Int32 = 0
    let began = Date()
    let rc = enoteCompanionCall(operation, planFd, credentialFd, 45_000, &state, &flags, &expires, &error)
    let ms = Int(Date().timeIntervalSince(began) * 1000)
    print("\(name) rc=\(rc) state=\(state) flags=\(flags) expires=\(expires > 0 ? "set" : "none") error=\(error) ms=\(ms)")
    fflush(stdout)
}

for step in CommandLine.arguments.dropFirst() {
    let parts = step.split(separator: ":", maxSplits: 2).map(String.init)
    switch parts[0] {
    case "canary":
        try! FileManager.default.createDirectory(atPath: vault, withIntermediateDirectories: true)
        try! "FAKE-VAULT-CANARY fictional data only\n".write(toFile: vault + "/fake-vault.canary", atomically: true, encoding: .utf8)
        print("canary written")
    case "status": call("status", 1)
    case "start": call("start", 2)
    case "stop": call("stop", 3)
    case "install":
        let plan = open(parts[1], O_RDONLY)
        let credential = open(parts[2], O_RDONLY)
        call("install", 4, plan, credential)
        close(plan)
        close(credential)
    case "install-fixtures":
        // FICTIONAL owner files inside this stand-in's own container (a sandboxed process can't
        // open files elsewhere unless the owner picks them in an OS panel, as the real app does).
        let dir = NSHomeDirectory() + "/fixtures"
        try! FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let plan = "{\"version\":2,\"ownerUid\":\(getuid()),\"agentUid\":\(getuid() + 1000),\"agentShortName\":\"enote-agent-fake-verify\",\"credentialExpirySeconds\":3600,\"projectHardSpendLimitCents\":100,\"projectHardSpendCurrency\":\"USD\",\"projectHardSpendInterval\":\"month\",\"brokerHost\":\"127.0.0.1\",\"brokerPort\":8789,\"providerModel\":\"gpt-6-luna\",\"providerScope\":\"api.responses.write\",\"bearerTokenFile\":\"/Users/Shared/encrypted-note-companion/agent-token\",\"grantExpiresAt\":null}"
        try! plan.write(toFile: dir + "/plan.json", atomically: true, encoding: .utf8)
        try! "OPENAI_AGENT_API_KEY=FAKE_OPENAI_AGENT_KEY_FOR_VERIFY_ONLY\n".write(toFile: dir + "/credential.env", atomically: true, encoding: .utf8)
        let planFd = open(dir + "/plan.json", O_RDONLY)
        let credentialFd = open(dir + "/credential.env", O_RDONLY)
        call("install", 4, planFd, credentialFd)
        close(planFd)
        close(credentialFd)
    case "sleep": Thread.sleep(forTimeInterval: Double(parts[1]) ?? 1)
    case "pid": print("pid=\(getpid())")
    default: print("unknown step")
    }
    fflush(stdout)
}
