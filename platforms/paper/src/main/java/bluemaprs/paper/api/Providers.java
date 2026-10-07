package bluemaprs.paper.api;

import bluemaprs.paper.skins.DefaultPlayerIconFactory;
import bluemaprs.paper.skins.MojangSkinProvider;
import de.bluecolored.bluemap.api.plugin.PlayerDisplayNameProvider;
import de.bluecolored.bluemap.api.plugin.PlayerIconFactory;
import de.bluecolored.bluemap.api.plugin.SkinProvider;

import java.util.Objects;

/** JVM-side callbacks of {@code BlueMapAPI.getPlugin()}; they outlive API generations like upstream's. */
public final class Providers {

    // TODO: skin updater (join → SkinProvider → PlayerIconFactory → AssetWrite playerheads/<uuid>.png per map)
    private volatile SkinProvider skinProvider = new MojangSkinProvider();
    private volatile PlayerIconFactory iconFactory = new DefaultPlayerIconFactory();
    private volatile PlayerDisplayNameProvider displayNameProvider;

    public Providers(PlayerDisplayNameProvider defaultDisplayName) {
        this.displayNameProvider = defaultDisplayName;
    }

    public SkinProvider getSkinProvider() {
        return skinProvider;
    }

    public void setSkinProvider(SkinProvider skinProvider) {
        this.skinProvider = Objects.requireNonNull(skinProvider, "skinProvider can not be null");
    }

    public PlayerIconFactory getPlayerMarkerIconFactory() {
        return iconFactory;
    }

    public void setPlayerMarkerIconFactory(PlayerIconFactory iconFactory) {
        this.iconFactory = Objects.requireNonNull(iconFactory, "playerMarkerIconFactory can not be null");
    }

    public PlayerDisplayNameProvider getPlayerDisplayNameProvider() {
        return displayNameProvider;
    }

    public void setPlayerDisplayNameProvider(PlayerDisplayNameProvider provider) {
        this.displayNameProvider = Objects.requireNonNull(provider, "playerNameProvider can not be null");
    }

}
