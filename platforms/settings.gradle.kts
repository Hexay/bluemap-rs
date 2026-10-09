pluginManagement {
    repositories {
        maven("https://maven.fabricmc.net/")
        gradlePluginPortal()
    }
}

rootProject.name = "bluemap-rs-platforms"

include(":common", ":paper", ":fabric", ":e2e-addon")
