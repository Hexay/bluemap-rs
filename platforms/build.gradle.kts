// shims report <upstream>+rs.<crate>, so addon version checks against BlueMap (`bluemap >=5`) keep passing
val crateVersion: String = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(file("../Cargo.toml").readText())!!.groupValues[1]

subprojects {
    group = "bluemaprs"
    version = "${property("bluemapVersion")}+rs.$crateVersion"
    extra["crateVersion"] = crateVersion

    tasks.withType<JavaCompile>().configureEach {
        options.encoding = "UTF-8"
    }
}
