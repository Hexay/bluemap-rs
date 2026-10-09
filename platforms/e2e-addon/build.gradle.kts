// test-only BlueMapAPI addon for tools/e2e_paper.py; never shipped
plugins {
    java
}

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
}

dependencies {
    compileOnly(project(":common"))
    compileOnly("io.papermc.paper:paper-api:1.21.11-R0.1-SNAPSHOT")
}

tasks.withType<JavaCompile>().configureEach {
    options.release.set(21)
}

tasks.jar {
    archiveBaseName.set("bluemap-e2e-addon")
    archiveVersion.set("")
    // no NMS: keep Paper from remapping the jar into plugins/.paper-remapped
    manifest.attributes("paperweight-mappings-namespace" to "mojang")
}
