// shims report <upstream>+rs.<crate>, so addon version checks against BlueMap (`bluemap >=5`) keep passing
// a release build passes its tag's version (tools/build_core.py `release_version`), which the core reports too
val crateVersion: String = (findProperty("releaseVersion") as String?) ?: Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(file("../Cargo.toml").readText())!!.groupValues[1]

subprojects {
    group = "bluemaprs"
    version = "${property("bluemapVersion")}+rs.$crateVersion"
    extra["crateVersion"] = crateVersion

    tasks.withType<JavaCompile>().configureEach {
        options.encoding = "UTF-8"
    }
}
