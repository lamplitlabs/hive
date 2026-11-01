// swift-tools-version: 6.0
// The PULSE Mac app: a native window around the console the hub serves, the
// same pages the browser shows. `scripts/build.sh` turns the executable into
// PULSE.app; README.md ("The Mac app") is the documentation.
import PackageDescription

let package = Package(
    name: "PulseMac",
    // 13.3 ships Safari 16.4's WebKit, the oldest engine the console's Next
    // export targets.
    platforms: [.macOS("13.3")],
    products: [
        .executable(name: "PULSE", targets: ["PULSE"]),
    ],
    targets: [
        // What decides: where PULSE lives, how its console runs, which links stay.
        .target(name: "PulseKit"),
        // The AppKit shell: windows, menus, the web view.
        .executableTarget(name: "PULSE", dependencies: ["PulseKit"]),
        .testTarget(name: "PulseKitTests", dependencies: ["PulseKit"]),
    ]
)
