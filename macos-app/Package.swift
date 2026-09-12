// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "MdxPDFApp",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "mdx-pdf-app", targets: ["MdxPDFApp"])
    ],
    targets: [
        .executableTarget(
            name: "MdxPDFApp",
            path: ".",
            exclude: ["AppIcon.icns", "AppIcon.png", "AppIcon.svg", "Info.plist", "build_app.sh"]
        )
    ],
    swiftLanguageModes: [.v5]
)
