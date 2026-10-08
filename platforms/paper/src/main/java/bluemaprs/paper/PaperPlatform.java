package bluemaprs.paper;

import bluemaprs.shim.Platform;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto.WorldInfo;
import io.papermc.paper.ServerBuildInfo;
import org.bukkit.Bukkit;
import org.bukkit.Material;
import org.bukkit.NamespacedKey;
import org.bukkit.World;
import org.bukkit.plugin.java.JavaPlugin;
import org.slf4j.Logger;

import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;

/** Facts about the running server that the core needs (upstream {@code BukkitPlugin}/{@code BukkitWorld}). */
final class PaperPlatform implements Platform {

    static final boolean IS_FOLIA = classExists("io.papermc.paper.threadedregions.RegionizedServer");

    private static final Method LEVEL_DIRECTORY = levelDirectoryMethod();

    private final JavaPlugin plugin;
    private final Logger log;
    private volatile boolean stopping;

    PaperPlatform(JavaPlugin plugin) {
        this.plugin = plugin;
        this.log = plugin.getSLF4JLogger();
    }

    /** The main thread is about to block in {@code onDisable} while the core shuts down. */
    void stopping() {
        stopping = true;
    }

    @Override
    public String id() {
        return "paper";
    }

    @Override
    public String displayName() {
        return "Paper";
    }

    @Override
    public String shimVersion() {
        return plugin.getPluginMeta().getVersion();
    }

    @Override
    public String minecraftVersion() {
        try {
            return ServerBuildInfo.buildInfo().minecraftVersionId();
        } catch (Throwable t) {
            return Bukkit.getMinecraftVersion();
        }
    }

    @Override
    public Path configFolder() {
        return plugin.getDataFolder().toPath().toAbsolutePath();
    }

    @Override
    public String modsFolder() {
        return "mods";
    }

    @Override
    public boolean folia() {
        return IS_FOLIA;
    }

    @Override
    public List<WorldInfo> worlds() {
        return Bukkit.getWorlds().stream().map(PaperPlatform::worldInfo).toList();
    }

    static WorldInfo worldInfo(World world) {
        String key = world.getKey().toString();
        World.Environment env = world.getEnvironment();
        String dimensionType = env == World.Environment.NORMAL ? "minecraft:overworld"
                : env == World.Environment.NETHER ? "minecraft:the_nether"
                : env == World.Environment.THE_END ? "minecraft:the_end"
                : null;
        return new WorldInfo(key, world.getName(), world.getUID().toString(),
                levelFolder(world).toAbsolutePath().toString(), key, dimensionType);
    }

    /** {@code BlueMapAPI.getWorld(Object)} lookups, as upstream {@code BukkitPlugin.getServerWorld}, plus keys. */
    @Override
    public Optional<String> serverWorldId(Object world) {
        World resolved = switch (world) {
            case World w -> w;
            case UUID uuid -> Bukkit.getWorld(uuid);
            case String s -> worldByString(s);
            default -> null;
        };
        return Optional.ofNullable(resolved).map(w -> w.getKey().toString());
    }

    /** As upstream {@code getDefaultBlockstates()}: every block {@code Material}'s default {@code BlockData}. */
    @Override
    public byte[] defaultBlockstates() {
        Map<String, String> states = new LinkedHashMap<>();
        try {
            for (Material material : Material.values()) {
                if (material.isLegacy() || !material.isBlock()) continue;
                try {
                    states.put(material.getKey().toString(), material.createBlockData().getAsString());
                } catch (RuntimeException e) {
                    log.debug("Failed to get the default blockstate for material '{}': {}", material, e.toString());
                }
            }
        } catch (Throwable t) {
            log.warn("Failed to dump the default blockstates: {}", t.toString());
        }
        return Msg.GSON.toJson(states).getBytes(StandardCharsets.UTF_8);
    }

    /** Upstream {@code BukkitWorld.persistWorldChanges}: main-thread {@code world.save()}; never on Folia. */
    @Override
    public CompletableFuture<Boolean> saveWorld(String worldId) {
        if (IS_FOLIA || stopping) return CompletableFuture.completedFuture(false);
        return CompletableFuture.supplyAsync(() -> {
            if (worldId == null) {
                Bukkit.getWorlds().forEach(World::save);
                return true;
            }
            World world = worldById(worldId);
            if (world == null) return false;
            world.save();
            return true;
        }, Bukkit.getScheduler().getMainThreadExecutor(plugin));
    }

    /** Folia has no global tick: unknown. */
    @Override
    public double averageTickMillis() {
        return IS_FOLIA ? 0 : Bukkit.getServer().getAverageTickTime();
    }

    private static World worldById(String id) {
        NamespacedKey key = NamespacedKey.fromString(id);
        return key == null ? null : Bukkit.getWorld(key);
    }

    private static World worldByString(String s) {
        World world = Bukkit.getWorld(s);
        if (world == null) {
            try {
                world = worldById(s);
            } catch (IllegalArgumentException ignored) {
                // not a key
            }
        }
        return world != null ? world : Bukkit.getWorld(s.substring(s.indexOf(':') + 1));
    }

    // 26.x: one level folder for all dimensions; 1.21: per-world folders (world_nether/DIM-1)
    private static Path levelFolder(World world) {
        if (LEVEL_DIRECTORY != null) {
            try {
                return (Path) LEVEL_DIRECTORY.invoke(Bukkit.getServer());
            } catch (ReflectiveOperationException | RuntimeException ignored) {
                // fall back
            }
        }
        return world.getWorldFolder().toPath();
    }

    private static Method levelDirectoryMethod() {
        try {
            return Bukkit.getServer().getClass().getMethod("getLevelDirectory");
        } catch (NoSuchMethodException | RuntimeException e) {
            return null;
        }
    }

    private static boolean classExists(String name) {
        try {
            Class.forName(name);
            return true;
        } catch (ClassNotFoundException e) {
            return false;
        }
    }

}
