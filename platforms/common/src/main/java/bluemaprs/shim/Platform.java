package bluemaprs.shim;

import bluemaprs.shim.ipc.Proto.WorldInfo;

import java.nio.file.Path;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;

/** What {@link ShimCore} needs from the server it runs in (upstream's {@code serverinterface.Server}). */
public interface Platform {

    /** {@code Hello.platform}, e.g. {@code paper}. */
    String id();

    /** For {@code /bluemap version}, e.g. {@code Paper}. */
    String displayName();

    String shimVersion();

    String minecraftVersion();

    /** Absolute; also holds the extracted core binary. */
    Path configFolder();

    /** Relative to the server folder, or null. */
    String modsFolder();

    boolean folia();

    /** Called off the server thread whenever the core (re)starts. */
    List<WorldInfo> worlds();

    /** {@code {"minecraft:stone": "minecraft:stone", …}} as UTF-8 JSON; called once. */
    byte[] defaultBlockstates();

    /** {@code BlueMapAPI.getWorld(Object)}: a platform world, key, name … → {@code WorldInfo.id}. */
    Optional<String> serverWorldId(Object world);

    /** Saves {@code worldId} ({@code WorldInfo.id}, null = all) on the server thread → whether it saved. */
    CompletableFuture<Boolean> saveWorld(String worldId);

}
