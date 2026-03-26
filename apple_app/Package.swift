// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "SubReader",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "SubReader", targets: ["SubReader"]),
    ],
    targets: [
        // ─── ReaderModels: Pure data models (zero dependencies) ──────────
        .target(
            name: "ReaderModels",
            path: "SubReader/Sources/Models",
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ]
        ),

        // ─── ReaderBridge: C-ABI bridge layer ────────────────────────────
        .systemLibrary(
            name: "CReaderCore",
            path: "SubReader/Vendor",
            pkgConfig: nil,
            providers: nil
        ),
        .target(
            name: "ReaderBridge",
            dependencies: ["CReaderCore", "ReaderModels"],
            path: "SubReader/Sources/Bridge",
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ],
            linkerSettings: [
                .unsafeFlags(["-L", "SubReader/Vendor"])
            ]
        ),

        // ─── Main App Target ─────────────────────────────────────────────
        .executableTarget(
            name: "SubReader",
            dependencies: ["ReaderBridge", "ReaderModels"],
            path: "SubReader/Sources",
            exclude: ["Models", "Bridge"],
            swiftSettings: [
                .enableExperimentalFeature("StrictConcurrency")
            ]
        ),

        // ─── Tests ───────────────────────────────────────────────────────
        .testTarget(
            name: "BridgeTests",
            dependencies: ["ReaderBridge", "ReaderModels"],
            path: "SubReader/Tests/BridgeTests"
        ),
        .testTarget(
            name: "ModelTests",
            dependencies: ["ReaderModels"],
            path: "SubReader/Tests/ModelTests"
        ),
        .testTarget(
            name: "PerformanceTests",
            dependencies: ["ReaderBridge", "ReaderModels"],
            path: "SubReader/Tests/PerformanceTests"
        ),
    ]
)
