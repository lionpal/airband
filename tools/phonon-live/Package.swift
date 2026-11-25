// swift-tools-version:6.0
// The phone's own listening (apps/apple/Sources/NeuralEngineListening.swift, linked in, not copied) built for
// this Mac as a library the core's test loads: `tools/phonon-live.sh` builds it and runs the test. Never part
// of the app. The package version is the app's (apps/apple/project.yml); the script refuses to run when they
// differ.
import PackageDescription

let package = Package(
    name: "phonon-live",
    platforms: [.macOS(.v15)],
    products: [
        .library(name: "PhononHost", type: .dynamic, targets: ["PhononHost"]),
    ],
    dependencies: [
        .package(url: "https://github.com/fermionresearch/phonon-coreml", exact: "1.1.2"),
    ],
    targets: [
        .target(
            name: "PhononHost",
            dependencies: [.product(name: "PhononCoreML", package: "phonon-coreml")],
            path: "Sources/PhononHost",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
