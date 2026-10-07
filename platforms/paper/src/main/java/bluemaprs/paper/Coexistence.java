package bluemaprs.paper;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Optional;
import java.util.regex.Pattern;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Detects upstream BlueMap next to us: both would claim {@code BlueMap} and render the same maps. */
final class Coexistence {

    private static final String UPSTREAM_MAIN = "de/bluecolored/bluemap/bukkit/BukkitPlugin.class";
    private static final String OWN_MAIN = BlueMapPaperPlugin.class.getName().replace('.', '/') + ".class";
    private static final Pattern NAME_BLUEMAP = Pattern.compile("^name:\\s*[\"']?BlueMap[\"']?\\s*$");

    private Coexistence() {}

    /** A jar in {@code pluginsFolder} that is upstream BlueMap (any jar named BlueMap that isn't bluemap-rs). */
    static Optional<Path> findUpstream(Path pluginsFolder) {
        try (DirectoryStream<Path> jars = Files.newDirectoryStream(pluginsFolder, "*.jar")) {
            for (Path jar : jars) {
                if (isUpstream(jar)) return Optional.of(jar);
            }
        } catch (IOException ignored) {
            // unreadable plugins folder: nothing to compare against
        }
        return Optional.empty();
    }

    private static boolean isUpstream(Path jar) {
        try (ZipFile zip = new ZipFile(jar.toFile())) {
            if (zip.getEntry(UPSTREAM_MAIN) != null) return true;
            if (zip.getEntry(OWN_MAIN) != null) return false;
            for (String descriptor : List.of("plugin.yml", "paper-plugin.yml")) {
                ZipEntry entry = zip.getEntry(descriptor);
                if (entry != null && declaresBlueMap(zip.getInputStream(entry))) return true;
            }
            return false;
        } catch (IOException e) {
            return false;
        }
    }

    private static boolean declaresBlueMap(InputStream in) throws IOException {
        try (BufferedReader reader = new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8))) {
            return reader.lines().anyMatch(line -> NAME_BLUEMAP.matcher(line).matches());
        }
    }

}
