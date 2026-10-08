package bluemaprs.shim.core;

import bluemaprs.shim.ipc.Msg;
import org.slf4j.Logger;

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.DigestInputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Locale;
import java.util.Map;

/**
 * Finds the core binary: {@code BLUEMAP_CORE} override, else the jar's {@code natives/<target>} entry extracted to
 * {@code <data>/bin/<target>/bluemap-core-<version>} and SHA-256 checked against {@code natives/manifest.json}.
 */
public final class CoreBinary {

    public static final String OVERRIDE = "BLUEMAP_CORE";

    private record Manifest(String coreVersion, Map<String, Target> targets) {}

    private record Target(String file, String sha256) {}

    private final Path dataFolder;
    private final Logger log;

    public CoreBinary(Path dataFolder, Logger log) {
        this.dataFolder = dataFolder;
        this.log = log;
    }

    /** {@code windows-x64|linux-x64|linux-arm64|linux-armv7|macos-x64|macos-arm64}, or null if unsupported. */
    public static String targetId() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String o = os.startsWith("windows") ? "windows" : os.startsWith("linux") ? "linux"
                : os.startsWith("mac") || os.contains("darwin") ? "macos" : null;
        String a = switch (arch) {
            case "amd64", "x86_64" -> "x64";
            case "aarch64", "arm64" -> "arm64";
            // 32-bit ARM JVMs report "arm"; the armv7 musl core runs on any ARMv7+ hard-float Linux
            case "arm", "armv7l", "armhf" -> "armv7";
            default -> null;
        };
        if (o == null || a == null) return null;
        String target = o + "-" + a;
        return switch (target) {
            case "windows-arm64", "windows-armv7", "macos-armv7" -> null;
            default -> target;
        };
    }

    /** The executable to spawn; extracts or repairs the bundled one as needed. */
    public Path resolve() throws CoreUnavailableException {
        String override = System.getProperty(OVERRIDE);
        if (override == null || override.isBlank()) override = System.getenv(OVERRIDE);
        if (override != null && !override.isBlank()) {
            Path path = Path.of(override).toAbsolutePath();
            if (!Files.isRegularFile(path))
                throw new CoreUnavailableException(OVERRIDE + " points to " + path + ", which is not a file.");
            log.info("Using the BlueMap core from " + OVERRIDE + ": " + path);
            return path;
        }

        String target = targetId();
        String platform = System.getProperty("os.name") + "/" + System.getProperty("os.arch");
        if (target == null)
            throw new CoreUnavailableException("BlueMap has no core binary for this platform (" + platform + ").");
        Manifest manifest = readManifest();
        Target entry = manifest == null || manifest.targets() == null ? null : manifest.targets().get(target);
        if (entry == null)
            throw new CoreUnavailableException("This BlueMap jar contains no core binary for " + target + " ("
                    + platform + "); download the jar for your platform or the universal jar.");

        try {
            return extract(target, manifest.coreVersion(), entry);
        } catch (IOException e) {
            throw new CoreUnavailableException("Failed to extract the BlueMap core to " + binFolder(target) + ": " + e, e);
        }
    }

    /** After a successful start: removes older extracted versions next to {@code current}. */
    public void deleteOtherVersions(Path current) {
        Path dir = current.getParent();
        if (dir == null || !current.startsWith(dataFolder.resolve("bin"))) return;
        try (DirectoryStream<Path> files = Files.newDirectoryStream(dir, "bluemap-core-*")) {
            for (Path file : files) {
                if (file.equals(current)) continue;
                try {
                    Files.deleteIfExists(file);
                } catch (IOException e) {
                    log.debug("Could not delete old core " + file + ": " + e.getMessage());
                }
            }
        } catch (IOException e) {
            log.debug("Could not list " + dir + ": " + e.getMessage());
        }
    }

    private Path binFolder(String target) {
        return dataFolder.resolve("bin").resolve(target);
    }

    private Path extract(String target, String version, Target entry) throws IOException, CoreUnavailableException {
        Path dir = binFolder(target);
        Files.createDirectories(dir);
        String ext = target.startsWith("windows") ? ".exe" : "";
        Path dest = dir.resolve("bluemap-core-" + version + ext);

        if (Files.isRegularFile(dest) && entry.sha256().equalsIgnoreCase(sha256(dest))) {
            makeExecutable(dest);
            return dest;
        }

        log.info("Extracting the BlueMap core " + version + " (" + target + ") to " + dest);
        Path tmp = Files.createTempFile(dir, "bluemap-core-", ".tmp");
        try {
            String actual;
            try (InputStream in = CoreBinary.class.getResourceAsStream("/" + entry.file())) {
                if (in == null)
                    throw new CoreUnavailableException("The core binary " + entry.file() + " is missing from this jar.");
                DigestInputStream digest = new DigestInputStream(in, sha256Digest());
                Files.copy(digest, tmp, StandardCopyOption.REPLACE_EXISTING);
                actual = HexFormat.of().formatHex(digest.getMessageDigest().digest());
            }
            if (!actual.equalsIgnoreCase(entry.sha256()))
                throw new CoreUnavailableException("The core binary " + entry.file() + " in this jar is corrupt "
                        + "(SHA-256 " + actual + ", expected " + entry.sha256() + "); re-download the jar.");
            makeExecutable(tmp);
            try {
                Files.move(tmp, dest, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(tmp, dest, StandardCopyOption.REPLACE_EXISTING);
            }
            return dest;
        } finally {
            Files.deleteIfExists(tmp);
        }
    }

    private static Manifest readManifest() throws CoreUnavailableException {
        try (InputStream in = CoreBinary.class.getResourceAsStream("/natives/manifest.json")) {
            if (in == null) return null;
            try (Reader reader = new InputStreamReader(in, StandardCharsets.UTF_8)) {
                return Msg.GSON.fromJson(reader, Manifest.class);
            }
        } catch (IOException | RuntimeException e) {
            throw new CoreUnavailableException("natives/manifest.json in this jar is unreadable: " + e, e);
        }
    }

    private static void makeExecutable(Path file) {
        //noinspection ResultOfMethodCallIgnored
        file.toFile().setExecutable(true, false);
    }

    private static String sha256(Path file) throws IOException {
        try (DigestInputStream in = new DigestInputStream(Files.newInputStream(file), sha256Digest())) {
            in.transferTo(OutputStream.nullOutputStream());
            return HexFormat.of().formatHex(in.getMessageDigest().digest());
        }
    }

    private static MessageDigest sha256Digest() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

}
