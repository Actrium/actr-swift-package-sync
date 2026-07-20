// swift-tools-version: 6.0
import Foundation
import PackageDescription

let env = ProcessInfo.processInfo.environment
let bindingsPath = env["ACTR_BINDINGS_PATH"] ?? "ActrBindings"
let overrideBinaryPath = env["ACTR_BINARY_PATH"]
let localBinaryPath = "ActrFFI.xcframework"
let releaseTag = env["ACTR_BINARY_TAG"] ?? "v0.5.3"
let remoteBinaryURL = "https://github.com/Actrium/actr-swift-package-sync/releases/download/\(releaseTag)/ActrFFI.xcframework.zip"
let remoteBinaryChecksum = env["ACTR_BINARY_CHECKSUM"] ?? "474225f6055179eb20e3c2114c9d6af07dbd69ae3b669674e13273bef5fe61bd"

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
