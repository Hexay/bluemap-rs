package bluemaprs.paper;

import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto.WorldInfo;
import io.papermc.paper.ServerBuildInfo;
import org.bukkit.Bukkit;
import org.bukkit.Material;
import org.bukkit.NamespacedKey;
import org.bukkit.World;

import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.logging.Logger;

/** Facts about the running server that the core needs (upstream {@code BukkitPlugin}/{@code BukkitWorld}). */
final class ServerInfo {

    static final boolean IS_FOLIA = classExists("io.papermc.paper.threadedregions.RegionizedServer");

    private static final Method LEVEL_DIRECTORY = levelDirectoryMethod();

    private ServerInfo() {}

    static String minecraftVersion() {
        try {
            return ServerBuildInfo.buildInfo().minecraftVersionId();
        } catch (Throwable t) {
            return Bukkit.getMinecraftVersion();
        }
    }

    static List<WorldInfo> worlds() {
        return Bukkit.getWorlds().stream().map(ServerInfo::worldInfo).toList();
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

    static World worldById(String id) {
        NamespacedKey key = NamespacedKey.fromString(id);
        return key == null ? null : Bukkit.getWorld(key);
    }

    /** {@code BlueMapAPI.getWorld(Object)} lookups, as upstream {@code BukkitPlugin.getServerWorld}, plus keys. */
    static Optional<String> serverWorldId(Object world) {
        World resolved = switch (world) {
            case World w -> w;
            case UUID uuid -> Bukkit.getWorld(uuid);
            case String s -> worldByString(s);
            default -> null;
        };
        return Optional.ofNullable(resolved).map(w -> w.getKey().toString());
    }

    /** {@code {"minecraft:stone": "minecraft:stone", …}} as upstream {@code getDefaultBlockstates()}. */
    static byte[] defaultBlockstates(Logger log) {
        Map<String, String> states = new LinkedHashMap<>();
        try {
            for (Material material : Material.values()) {
                if (material.isLegacy() || !material.isBlock()) continue;
                try {
                    states.put(material.getKey().toString(), material.createBlockData().getAsString());
                } catch (RuntimeException e) {
                    log.fine("Failed to get the default blockstate for material '" + material + "': " + e);
                }
            }
        } catch (Throwable t) {
            log.warning("Failed to dump the default blockstates: " + t);
        }
        return Msg.GSON.toJson(states).getBytes(StandardCharsets.UTF_8);
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
