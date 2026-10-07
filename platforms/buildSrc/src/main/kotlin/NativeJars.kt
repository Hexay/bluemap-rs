import org.gradle.api.Project
import org.gradle.api.Task
import org.gradle.api.file.DuplicatesStrategy
import org.gradle.api.tasks.TaskProvider
import org.gradle.api.tasks.bundling.AbstractArchiveTask
import org.gradle.api.tasks.bundling.Jar
import org.gradle.kotlin.dsl.extra
import org.gradle.kotlin.dsl.register
import java.io.File
import java.security.MessageDigest

/** Core binary per plugin target, staged in platforms/natives/<target>/ by tools/build_core.py (docs/13 §5). */
private val KNOWN_TARGETS = linkedMapOf(
    "windows-x64" to "bluemap-core.exe",
    "linux-x64" to "bluemap-core",
    "linux-arm64" to "bluemap-core",
    "linux-armv7" to "bluemap-core",
    "macos-x64" to "bluemap-core",
    "macos-arm64" to "bluemap-core",
)

private fun sha256(f: File): String =
    MessageDigest.getInstance("SHA-256").digest(f.readBytes()).joinToString("") { "%02x".format(it) }

/**
 * Registers `jar-<target>` per staged target, `jar-universal` and `allJars`: the [shim] jar plus `natives/<target>/`
 * and the `natives/manifest.json` that `CoreBinary` verifies. Missing targets are skipped.
 */
fun Project.registerNativeJars(
    baseName: String,
    shim: TaskProvider<out AbstractArchiveTask>,
    manifestAttributes: Map<String, String> = emptyMap(),
): TaskProvider<Task> {
    val crateVersion = extra["crateVersion"] as String
    val coreVersion = (findProperty("coreVersion") as String?) ?: crateVersion
    val natives = rootProject.file("natives")
    val present = KNOWN_TARGETS.filter { (target, file) -> File(natives, "$target/$file").isFile }

    fun manifest(variant: String, targets: Map<String, String>): TaskProvider<Task> {
        val outDir = layout.buildDirectory.dir("generated/natives-manifest/$variant")
        val files = targets.mapValues { (target, file) -> File(natives, "$target/$file") }
        return tasks.register("nativesManifest-$variant") {
            inputs.files(files.values)
            inputs.property("coreVersion", coreVersion)
            outputs.dir(outDir)
            doLast {
                val entries = files.entries.joinToString(",\n") { (target, f) ->
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

    fun distJar(variant: String, targets: Map<String, String>): TaskProvider<Jar> {
        val manifestTask = manifest(variant, targets)
        return tasks.register<Jar>("jar-$variant") {
            group = "build"
            description = "$baseName jar with the $variant core binaries"
            dependsOn(shim)
            archiveBaseName.set(baseName)
            archiveVersion.set(crateVersion)
            archiveClassifier.set(variant)
            destinationDirectory.set(layout.buildDirectory.dir("libs"))
            duplicatesStrategy = DuplicatesStrategy.EXCLUDE
            if (manifestAttributes.isNotEmpty()) manifest.attributes(manifestAttributes)
            from(shim.flatMap { it.archiveFile }.map { zipTree(it) }) { exclude("META-INF/MANIFEST.MF") }
            from(manifestTask)
            targets.forEach { (target, file) -> from(File(natives, "$target/$file")) { into("natives/$target") } }
        }
    }

    val jars = present.map { (target, file) -> distJar(target, mapOf(target to file)) } + distJar("universal", present)
    return tasks.register("allJars") {
        group = "build"
        description = "Builds the per-target and the universal $baseName jars"
        dependsOn(jars)
    }
}
