// swift-tools-version: 6.0
import Foundation
import PackageDescription

let env = ProcessInfo.processInfo.environment
let bindingsPath = env["ACTR_BINDINGS_PATH"] ?? "ActrBindings"
let overrideBinaryPath = env["ACTR_BINARY_PATH"]
let localBinaryPath = "ActrFFI.xcframework"
let releaseTag = env["ACTR_BINARY_TAG"] ?? "v0.4.18"
let remoteBinaryURL = "https://github.com/Actrium/actr-swift-package-sync/releases/download/\(releaseTag)/ActrFFI.xcframework.zip"
let remoteBinaryChecksum = env["ACTR_BINARY_CHECKSUM"] ?? "b167659c524c759e094debad2ce81ac4ff19d18b003558f97122be03566e87ef"

let manifestDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let localBinaryAbsolutePath = URL(fileURLWithPath: localBinaryPath, relativeTo: URL(fileURLWithPath: manifestDir)).path

func binaryPathRelativeToPackageRoot(_ path: String) -> String? {
    if path.hasPrefix("/") {
        let prefix = manifestDir.hasSuffix("/") ? manifestDir : "\(manifestDir)/"
        guard path.hasPrefix(prefix) else { return nil }
        return String(path.dropFirst(prefix.count))
    }
    return path
}

let actrBinaryTarget: Target
if let overrideBinaryPath {
    if let relativeBinaryPath = binaryPathRelativeToPackageRoot(overrideBinaryPath) {
        actrBinaryTarget = .binaryTarget(
            name: "ActrFFILib",
            path: relativeBinaryPath
        )
    } else {
        actrBinaryTarget = .binaryTarget(
            name: "ActrFFILib",
            url: remoteBinaryURL,
            checksum: remoteBinaryChecksum
        )
    }
} else if FileManager.default.fileExists(atPath: localBinaryAbsolutePath) {
    actrBinaryTarget = .binaryTarget(
        name: "ActrFFILib",
        path: localBinaryPath
    )
} else {
    actrBinaryTarget = .binaryTarget(
        name: "ActrFFILib",
        url: remoteBinaryURL,
        checksum: remoteBinaryChecksum
    )
}

let package = Package(
    name: "actr",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
    ],
    products: [
        .library(
            name: "Actr",
            targets: ["Actr"]
        ),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-protobuf.git", .upToNextMinor(from: "1.32.0")),
    ],
    targets: [
        actrBinaryTarget,
        .target(
            name: "ActrFFI",
            path: bindingsPath,
            sources: ["actrFFI.c"],
            publicHeadersPath: "include"
        ),
        .target(
            name: "ActrBindings",
            dependencies: ["ActrFFI", "ActrFFILib"],
            path: bindingsPath,
            exclude: ["actrFFI.c"],
            sources: ["Actr.swift"]
        ),
        .target(
            name: "Actr",
            dependencies: [
                "ActrFFI",
                "ActrBindings",
                "ActrFFILib",
                .product(name: "SwiftProtobuf", package: "swift-protobuf"),
            ],
            path: "Sources/Actr"
        ),
    ]
)
