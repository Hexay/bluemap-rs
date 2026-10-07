package bluemaprs.fabric;

import bluemaprs.shim.Platform;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto.WorldInfo;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.storage.LevelResource;

import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;

/** Facts about the running dedicated server (upstream {@code FabricMod}/{@code FabricWorld}). */
final class FabricPlatform implements Platform {

    private final String version;
    private volatile MinecraftServer server;
    private volatile boolean stopping;

    FabricPlatform(String version) {
        this.version = version;
    }

    void started(MinecraftServer server) {
        this.server = server;
    }

    /** The server thread is about to block in {@code SERVER_STOPPING} while the core shuts down. */
    void stopping() {
        stopping = true;
    }

    @Override
    public String id() {
        return "fabric";
    }

    @Override
    public String displayName() {
        return "Fabric";
    }

    @Override
    public String shimVersion() {
        return version;
    }

    @Override
    public String minecraftVersion() {
        return SharedConstants.getCurrentVersion().id();
    }

    /** Relative to the server folder, as upstream. */
    @Override
    public Path configFolder() {
        return Path.of("config", "bluemap").toAbsolutePath();
    }

    @Override
    public String modsFolder() {
        return "mods";
    }

    @Override
    public boolean folia() {
        return false;
    }

    @Override
    public List<WorldInfo> worlds() {
        List<WorldInfo> worlds = new ArrayList<>();
        for (ServerLevel level : server.getAllLevels()) worlds.add(worldInfo(level));
        return worlds;
    }

    /** Every dimension shares the level folder; the core tells them apart by {@code dimension}. */
    static WorldInfo worldInfo(ServerLevel level) {
        MinecraftServer server = level.getServer();
        String id = worldId(level);
        Path folder = server.getServerDirectory().resolve(server.getWorldPath(LevelResource.ROOT));
        String dimensionType = level.dimensionTypeRegistration().unwrapKey()
                .map(key -> key.identifier().toString()).orElse(null);
        return new WorldInfo(id, id, id, folder.toAbsolutePath().normalize().toString(), id, dimensionType);
    }

    static String worldId(Level level) {
        return level.dimension().identifier().toString();
    }

    /** {@code BlueMapAPI.getWorld(Object)}, as upstream {@code FabricMod.getServerWorld}. */
    @Override
    @SuppressWarnings("unchecked")
    public Optional<String> serverWorldId(Object world) {
        MinecraftServer s = server;
        if (s == null) return Optional.empty();
        if (world instanceof String string) {
            Identifier id = Identifier.tryParse(string);
            if (id != null) world = s.getLevel(ResourceKey.create(Registries.DIMENSION, id));
        }
        if (world instanceof ResourceKey<?> key) {
            try {
                world = s.getLevel((ResourceKey<Level>) key);
            } catch (ClassCastException ignored) {
                // not a dimension key
            }
        }
        return world instanceof ServerLevel level ? Optional.of(worldId(level)) : Optional.empty();
    }

    /** Every registered block (modded ones too), as upstream {@code FabricMod.getDefaultBlockstates}. */
    @Override
    public byte[] defaultBlockstates() {
        Map<String, String> states = new LinkedHashMap<>();
        for (Block block : BuiltInRegistries.BLOCK) {
            String id = BuiltInRegistries.BLOCK.getKey(block).toString();
            List<String> properties = new ArrayList<>();
            block.defaultBlockState().getValues()
                    .forEach(value -> properties.add(value.property().getName() + "=" + value.valueName()));
            properties.sort(null);
            states.put(id, properties.isEmpty() ? id : id + "[" + String.join(",", properties) + "]");
        }
        return Msg.GSON.toJson(states).getBytes(StandardCharsets.UTF_8);
    }

    /** Upstream {@code FabricWorld.persistWorldChanges}: {@code level.save} on the server thread. */
    @Override
    public CompletableFuture<Boolean> saveWorld(String worldId) {
        MinecraftServer s = server;
        if (s == null || stopping) return CompletableFuture.completedFuture(false);
        return CompletableFuture.supplyAsync(() -> {
            boolean saved = false;
            for (ServerLevel level : s.getAllLevels()) {
                if (worldId != null && !worldId.equals(worldId(level))) continue;
                level.save(null, true, false);
                saved = true;
            }
            return saved;
        }, s);
    }

}
