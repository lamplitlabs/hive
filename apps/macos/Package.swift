// swift-tools-version: 6.0
// The Hive Mac app: a native window around the console the hub serves, the
// same pages the browser shows. `scripts/build.sh` turns the executable into
// Hive.app; docs/mac-app.md is the documentation.
import PackageDescription

let package = Package(
    name: "HiveMac",
    // 13.3 ships Safari 16.4's WebKit, the oldest engine the console's Next
    // export targets.
    platforms: [.macOS("13.3")],
    products: [
        .executable(name: "Hive", targets: ["Hive"]),
    ],
    targets: [
        // What decides: where Hive lives, how its console runs, which links stay.
        .target(name: "HiveKit"),
        // The AppKit shell: windows, menus, the web view.
        .executableTarget(name: "Hive", dependencies: ["HiveKit"]),
        .testTarget(name: "HiveKitTests", dependencies: ["HiveKit"]),
    ]
)
