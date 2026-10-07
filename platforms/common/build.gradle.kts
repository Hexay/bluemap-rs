plugins {
    `java-library`
}

repositories {
    mavenCentral()
    maven("https://repo.bluecolored.de/releases")
    maven("https://libraries.minecraft.net")
}

// bluemap-api 2.8.1 is published as Java 25 class files: recompile its official sources jar for Java 21
val bluemapApiSources: Configuration by configurations.creating {
    isCanBeConsumed = false
    isTransitive = false
    attributes {
        attribute(Category.CATEGORY_ATTRIBUTE, objects.named(Category.DOCUMENTATION))
        attribute(DocsType.DOCS_TYPE_ATTRIBUTE, objects.named(DocsType.SOURCES))
        attribute(Bundling.BUNDLING_ATTRIBUTE, objects.named(Bundling.EXTERNAL))
        attribute(Usage.USAGE_ATTRIBUTE, objects.named(Usage.JAVA_RUNTIME))
    }
}

val extractBluemapApi by tasks.registering(Sync::class) {
    from(bluemapApiSources.elements.map { files -> files.map { zipTree(it) } }) { exclude("META-INF/**") }
    into(layout.buildDirectory.dir("generated/bluemap-api"))
}

val bluemapApi: SourceSet by sourceSets.creating {
    java.srcDir(extractBluemapApi.map { it.destinationDir })
    resources.srcDir(extractBluemapApi.map { it.destinationDir })
    resources.include("**/*.json")
}

// unrelocated in every shim jar: addons link against de.bluecolored.bluemap.api
val bluemapApiJar by tasks.registering(Jar::class) {
    archiveClassifier.set("bluemap-api")
    from(bluemapApi.output)
}

dependencies {
    bluemapApiSources("de.bluecolored:bluemap-api:2.8.1")
    "bluemapApiCompileOnly"("com.google.code.gson:gson:2.11.0")
    "bluemapApiCompileOnly"("com.flowpowered:flow-math:1.0.3")
    "bluemapApiCompileOnly"("org.jetbrains:annotations:26.0.2")

    api(files(bluemapApiJar))

    // provided by every server (Paper and Minecraft itself)
    compileOnly("com.google.code.gson:gson:2.11.0")
    compileOnly("com.flowpowered:flow-math:1.0.3")
    compileOnly("com.mojang:brigadier:1.3.10")
    compileOnly("org.slf4j:slf4j-api:2.0.17")

    testImplementation(platform("org.junit:junit-bom:5.13.4"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
    testImplementation("com.google.code.gson:gson:2.11.0")
}

tasks.withType<JavaCompile>().configureEach {
    options.release.set(21)
}

tasks.test {
    useJUnitPlatform()
}

tasks.processResources {
    from("../../crates/bm-cli/src/plugin/commands.json")
    // MIT: ships upstream-derived code (logger, skins, BlueMapAPI build) — carry both copyright notices
    from("../../LICENSE") { into("META-INF") }
}
