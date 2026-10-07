plugins {
    // the no-remap Loom: Minecraft 26.x ships unobfuscated
    id("net.fabricmc.fabric-loom") version "1.18.3"
}

// compiled against the oldest supported version, as upstream 5.28 (fabric.mod.json allows 26.1–26.3)
val minecraftVersion = "26.1"
val fabricLoaderVersion = "0.18.4"
val fabricApiVersion = "0.144.0+26.1"
val permissionsApi = "me.lucko:fabric-permissions-api:0.7.0"
val flowMath = "com.flowpowered:flow-math:1.0.3"

repositories {
    mavenCentral()
}

// :common's classes and its BlueMapAPI build go into the mod jar itself (unrelocated)
val bundled: Configuration by configurations.creating {
    isCanBeConsumed = false
    attributes {
        attribute(Usage.USAGE_ATTRIBUTE, objects.named(Usage.JAVA_RUNTIME))
        attribute(Category.CATEGORY_ATTRIBUTE, objects.named(Category.LIBRARY))
        attribute(LibraryElements.LIBRARY_ELEMENTS_ATTRIBUTE, objects.named(LibraryElements.JAR))
        attribute(Bundling.BUNDLING_ATTRIBUTE, objects.named(Bundling.EXTERNAL))
    }
}

dependencies {
    minecraft("com.mojang:minecraft:$minecraftVersion")
    implementation("net.fabricmc:fabric-loader:$fabricLoaderVersion")
    implementation("net.fabricmc.fabric-api:fabric-api:$fabricApiVersion")

    implementation(project(":common"))
    bundled(project(":common"))
    implementation(permissionsApi)
    implementation(flowMath)

    // jar-in-jar: Fabric dedupes them with other mods nesting the same libraries
    include(permissionsApi)
    include(flowMath)
}

tasks.withType<JavaCompile>().configureEach {
    options.release.set(25)
}

tasks.processResources {
    val replacements = mapOf("version" to version, "loader" to fabricLoaderVersion)
    inputs.properties(replacements)
    filesMatching("fabric.mod.json") { expand(replacements) }
}

// mod without natives; dev runs use it with BLUEMAP_CORE
tasks.jar {
    archiveBaseName.set("bluemap-rs-fabric")
    archiveVersion.set(project.extra["crateVersion"] as String)
    archiveClassifier.set("shim")
    destinationDirectory.set(layout.buildDirectory.dir("shim"))
    from(bundled.elements.map { files -> files.map { zipTree(it) } }) { exclude("META-INF/MANIFEST.MF") }
}

val allJars = registerNativeJars("bluemap-rs-fabric", tasks.jar)

tasks.assemble { dependsOn(allJars) }
