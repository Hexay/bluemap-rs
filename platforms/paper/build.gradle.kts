import java.security.MessageDigest

plugins {
    java
    id("com.gradleup.shadow") version "9.6.1"
}

group = "bluemaprs"
version = "0.1.0"

val coreVersion = (findProperty("coreVersion") as String?) ?: version.toString()

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
    maven("https://repo.bluecolored.de/releases")
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

dependencies {
    bluemapApiSources("de.bluecolored:bluemap-api:2.8.1")
    "bluemapApiCompileOnly"("com.google.code.gson:gson:2.11.0")
    "bluemapApiCompileOnly"("com.flowpowered:flow-math:1.0.3")
    "bluemapApiCompileOnly"("org.jetbrains:annotations:26.0.2")

    // 26.x API jars are Java 25 class files; 1.21.11 compiles for Java 21 and runs on 26.x
    compileOnly("io.papermc.paper:paper-api:1.21.11-R0.1-SNAPSHOT")
    compileOnly("com.flowpowered:flow-math:1.0.3")
    compileOnly(bluemapApi.output)

    implementation("org.bstats:bstats-bukkit:3.2.1")

    testImplementation(platform("org.junit:junit-bom:5.13.4"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
    testImplementation("com.google.code.gson:gson:2.11.0")
}

tasks.withType<JavaCompile>().configureEach {
    options.release.set(21)
    options.encoding = "UTF-8"
}

tasks.test {
    useJUnitPlatform()
}

tasks.processResources {
    inputs.property("version", version)
    filesMatching("plugin.yml") { expand("version" to version) }
    from("../../crates/bm-cli/src/plugin/commands.json")
    // MIT: ships upstream-derived code (logger, skins, BlueMapAPI build) — carry both copyright notices
    from("../../LICENSE") { into("META-INF") }
}

tasks.jar {
    archiveClassifier.set("plain")
    destinationDirectory.set(layout.buildDirectory.dir("tmp/plain"))
}

// shim without natives; dev runs use it with BLUEMAP_CORE
tasks.shadowJar {
    archiveClassifier.set("shim")
    destinationDirectory.set(layout.buildDirectory.dir("shim"))
    from(bluemapApi.output)
    relocate("org.bstats", "bluemaprs.paper.bstats")
    exclude("META-INF/maven/**")
}

// TODO: CI builds the other targets (cargo-zigbuild musl, macOS)
val knownTargets = linkedMapOf(
    "windows-x64" to "bluemap-core.exe",
    "linux-x64" to "bluemap-core",
    "linux-arm64" to "bluemap-core",
    "macos-x64" to "bluemap-core",
    "macos-arm64" to "bluemap-core",
)
val presentTargets = knownTargets.filter { (target, file) -> file("natives/$target/$file").isFile }

fun sha256(f: File): String =
    MessageDigest.getInstance("SHA-256").digest(f.readBytes()).joinToString("") { "%02x".format(it) }

fun registerManifest(variant: String, targets: Map<String, String>): TaskProvider<Task> {
    val outDir = layout.buildDirectory.dir("generated/natives-manifest/$variant")
    val natives = targets.mapValues { (target, file) -> file("natives/$target/$file") }
    return tasks.register("nativesManifest-$variant") {
        inputs.files(natives.values)
        inputs.property("coreVersion", coreVersion)
        outputs.dir(outDir)
        doLast {
            val entries = natives.entries.joinToString(",\n") { (target, f) ->
                "    \"$target\": {\"file\": \"natives/$target/${f.name}\", \"sha256\": \"${sha256(f)}\"}"
            }
            val json = "{\n  \"coreVersion\": \"$coreVersion\",\n  \"targets\": {\n$entries\n  }\n}\n"
            outDir.get().file("natives/manifest.json").asFile.apply {
                parentFile.mkdirs()
                writeText(json)
            }
        }
    }
}

fun registerDistJar(variant: String, targets: Map<String, String>): TaskProvider<Jar> {
    val manifestTask = registerManifest(variant, targets)
    val shim = tasks.shadowJar.flatMap { it.archiveFile }
    return tasks.register<Jar>("jar-$variant") {
        group = "build"
        description = "Plugin jar with the $variant core binaries"
        dependsOn(tasks.shadowJar)
        archiveBaseName.set("bluemap-rs-paper")
        archiveClassifier.set(variant)
        destinationDirectory.set(layout.buildDirectory.dir("libs"))
        duplicatesStrategy = DuplicatesStrategy.EXCLUDE
        // no NMS: keep Paper from remapping the jar into plugins/.paper-remapped
        manifest.attributes("paperweight-mappings-namespace" to "mojang")
        from(shim.map { zipTree(it) }) { exclude("META-INF/MANIFEST.MF") }
        from(manifestTask)
        targets.forEach { (target, file) -> from("natives/$target/$file") { into("natives/$target") } }
    }
}

val distJars = presentTargets.map { (target, file) -> registerDistJar(target, mapOf(target to file)) } +
    registerDistJar("universal", presentTargets)

val allJars by tasks.registering {
    group = "build"
    description = "Builds the per-target and the universal plugin jars"
    dependsOn(distJars)
}

tasks.assemble { dependsOn(allJars) }
