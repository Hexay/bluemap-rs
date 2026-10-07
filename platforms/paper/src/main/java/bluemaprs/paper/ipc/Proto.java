package bluemaprs.paper.ipc;

import java.util.List;

/** Mirrors of bm-ipc's {@code msg.rs} structs; Gson omits null fields, which serde reads as {@code None}. */
public final class Proto {

    private Proto() {}

    public record WorldInfo(String id, String name, String uuid, String folder, String dimension, String dimensionType) {}

    public record PlayerInfo(String uuid, String name, String world, double x, double y, double z,
                             double pitch, double yaw, int skyLight, int blockLight,
                             boolean sneaking, boolean invisible, boolean vanished, String gamemode) {}

    public record CommandSender(String kind, String name, String world, double[] position, List<String> permissions) {}

    public record Hello(int protocol, String platform, String mcVersion, String shimVersion, String configFolder,
                        String modsFolder, Boolean metrics, boolean folia, long maxMemoryMib, List<WorldInfo> worlds) {}

    public record CoreWorld(String id, String saveFolder, String serverWorld) {}

    public record MapInfo(String id, String name, String world, int[] tileSize, int[] tileOffset, boolean frozen,
                          String configMarkers) {}

    public record PluginInfo(boolean livePlayerMarkers, boolean skinDownload, int playerRenderLimit, boolean metrics) {}

    public record StateInfo(List<String> frozenMaps, List<String> hiddenPlayers, boolean renderThreadsRunning) {

        public static final StateInfo EMPTY = new StateInfo(List.of(), List.of(), false);

        public StateInfo {
            frozenMaps = frozenMaps == null ? List.of() : List.copyOf(frozenMaps);
            hiddenPlayers = hiddenPlayers == null ? List.of() : List.copyOf(hiddenPlayers);
        }

    }

    public record ReadyInfo(String coreVersion, String compatVersion, String webroot, List<CoreWorld> worlds,
                            List<MapInfo> maps, List<String> storages, PluginInfo plugin, StateInfo state) {

        public ReadyInfo {
            worlds = worlds == null ? List.of() : List.copyOf(worlds);
            maps = maps == null ? List.of() : List.copyOf(maps);
            storages = storages == null ? List.of() : List.copyOf(storages);
            if (state == null) state = StateInfo.EMPTY;
        }

    }

    public record NotReady(String reason, String message) {}

}
