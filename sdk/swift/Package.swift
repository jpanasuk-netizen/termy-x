// swift-tools-version: 6.0
import PackageDescription

// libtermy is built by Cargo, not SwiftPM. `scripts/build-libtermy.sh` stages it
// in `lib/`; set TERMY_LIBRARY_PATH to link a library from somewhere else.
let libraryPath = Context.environment["TERMY_LIBRARY_PATH"]
    ?? "\(Context.packageDirectory)/lib"

let linkLibtermy: [LinkerSetting] = [
    .unsafeFlags(["-L", libraryPath, "-Xlinker", "-rpath", "-Xlinker", libraryPath]),
]

let package = Package(
    name: "TermySDK",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "Termy", targets: ["Termy"]),
        .library(name: "TermyAppKit", targets: ["TermyAppKit"]),
    ],
    targets: [
        .systemLibrary(name: "CTermy", path: "Sources/CTermy"),
        .target(
            name: "Termy",
            dependencies: ["CTermy"],
            linkerSettings: linkLibtermy
        ),
        .target(
            name: "TermyAppKit",
            dependencies: ["Termy"]
        ),
        .executableTarget(
            name: "TermyDemo",
            dependencies: ["Termy", "TermyAppKit"],
            path: "Examples/TermyDemo"
        ),
        .executableTarget(
            name: "TermyHeadless",
            dependencies: ["Termy"],
            path: "Examples/TermyHeadless"
        ),
        .testTarget(
            name: "TermyTests",
            dependencies: ["Termy"],
            linkerSettings: linkLibtermy
        ),
        .testTarget(
            name: "TermyAppKitTests",
            dependencies: ["Termy", "TermyAppKit"],
            linkerSettings: linkLibtermy
        ),
    ]
)
