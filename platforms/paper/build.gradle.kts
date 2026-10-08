plugins {
    java
    id("com.gradleup.shadow") version "9.6.1"
}

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
}

dependencies {
    implementation(project(":common"))

    // 26.x API jars are Java 25 class files; 1.21.11 compiles for Java 21 and runs on 26.x
    compileOnly("io.papermc.paper:paper-api:1.21.11-R0.1-SNAPSHOT")

    implementation("org.bstats:bstats-bukkit:3.2.1")
}

tasks.withType<JavaCompile>().configureEach {
    options.release.set(21)
}

tasks.processResources {
    inputs.property("version", version)
    filesMatching("plugin.yml") { expand("version" to version) }
}

tasks.jar {
    archiveClassifier.set("plain")
    destinationDirectory.set(layout.buildDirectory.dir("tmp/plain"))
}

// shim without natives; dev runs use it with BLUEMAP_CORE
tasks.shadowJar {
    archiveBaseName.set("bluemap-rs-paper")
    archiveVersion.set(project.extra["crateVersion"] as String)
    archiveClassifier.set("shim")
    destinationDirectory.set(layout.buildDirectory.dir("shim"))
    relocate("org.bstats", "bluemaprs.paper.bstats")
    exclude("META-INF/maven/**")
}

// no NMS: keep Paper from remapping the jar into plugins/.paper-remapped
val allJars = registerNativeJars("bluemap-rs-paper", tasks.shadowJar, mapOf("paperweight-mappings-namespace" to "mojang"))

tasks.assemble { dependsOn(allJars) }
