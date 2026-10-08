package bluemaprs.fabric;

import java.io.IOException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;
import java.util.zip.ZipFile;

/**
 * Upstream BlueMap's Fabric jar in {@code mods/}. Both are mod id {@code bluemap}, and Fabric Loader silently loads
 * only one of them: when it picks upstream we never run; when it picks us, this makes us stand down (docs/16).
 */
final class Coexistence {

    private static final String UPSTREAM_MAIN = "de/bluecolored/bluemap/fabric/FabricMod.class";

    private Coexistence() {}

    static Optional<Path> findUpstream(Path modsFolder) {
        try (DirectoryStream<Path> jars = Files.newDirectoryStream(modsFolder, "*.jar")) {
            for (Path jar : jars) {
                if (isUpstream(jar)) return Optional.of(jar);
            }
        } catch (IOException ignored) {
            // no readable mods folder: nothing to compare against
        }
        return Optional.empty();
    }

    private static boolean isUpstream(Path jar) {
        try (ZipFile zip = new ZipFile(jar.toFile())) {
            return zip.getEntry(UPSTREAM_MAIN) != null;
        } catch (IOException e) {
            return false;
        }
    }

}
