// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "HerdrDesk",
    platforms: [.macOS("15.0")],
    products: [
        .library(name: "HerdrKit", targets: ["HerdrKit"]),
        .executable(name: "HerdrDesk", targets: ["HerdrDesk"]),
        .executable(name: "flockbook-notify", targets: ["flockbook-notify"]),
        .executable(name: "flockbook-statusline", targets: ["flockbook-statusline"]),
        .executable(name: "flockbook-modfeed", targets: ["flockbook-modfeed"]),
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", from: "1.9.0"),
        // Auto-update. Exact: release.yml signs updates with this version's sign_update.
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.6"),
    ],
    targets: [
        .target(name: "HerdrKit", exclude: ["README.md"]),
        .executableTarget(
            name: "HerdrDesk",
            dependencies: [
                "HerdrKit",
                .product(name: "SwiftTerm", package: "SwiftTerm"),
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            // Sparkle.framework sits in Contents/Frameworks of the app bundle (scripts/bundle.sh).
            linkerSettings: [.unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"])]
        ),
        .executableTarget(name: "HerdrKitChecks", dependencies: ["HerdrKit", "ModFeedKit"]),
        .executableTarget(name: "flockbook-notify", dependencies: ["HerdrKit"]),
        .executableTarget(name: "flockbook-statusline", dependencies: ["HerdrKit"]),
        // The herdr-app mod's sidecar (JSON Lines on stdout). ModFeedKit holds its logic, so the checks can test it.
        .target(name: "ModFeedKit", dependencies: ["HerdrKit"]),
        .executableTarget(name: "flockbook-modfeed", dependencies: ["HerdrKit", "ModFeedKit"]),
        .testTarget(
            name: "HerdrKitTests",
            dependencies: ["HerdrKit"],
            resources: [.copy("Fixtures")],
            swiftSettings: [.unsafeFlags(["-F", "/Library/Developer/CommandLineTools/Library/Developer/Frameworks"])],
            linkerSettings: [.unsafeFlags(["-F", "/Library/Developer/CommandLineTools/Library/Developer/Frameworks", "-Xlinker", "-rpath", "-Xlinker", "/Library/Developer/CommandLineTools/Library/Developer/Frameworks"])]
        ),
    ]
)
