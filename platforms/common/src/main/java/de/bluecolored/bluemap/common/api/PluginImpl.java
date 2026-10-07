package de.bluecolored.bluemap.common.api;

import bluemaprs.shim.api.Providers;
import de.bluecolored.bluemap.api.plugin.PlayerDisplayNameProvider;
import de.bluecolored.bluemap.api.plugin.PlayerIconFactory;
import de.bluecolored.bluemap.api.plugin.SkinProvider;

public class PluginImpl implements de.bluecolored.bluemap.api.plugin.Plugin {

    private final Providers providers;

    PluginImpl(Providers providers) {
        this.providers = providers;
    }

    @Override
    public SkinProvider getSkinProvider() {
        return providers.getSkinProvider();
    }

    @Override
    public void setSkinProvider(SkinProvider skinProvider) {
        providers.setSkinProvider(skinProvider);
    }

    @Override
    public PlayerIconFactory getPlayerMarkerIconFactory() {
        return providers.getPlayerMarkerIconFactory();
    }

    @Override
    public void setPlayerMarkerIconFactory(PlayerIconFactory playerMarkerIconFactory) {
        providers.setPlayerMarkerIconFactory(playerMarkerIconFactory);
    }

    @Override
    public PlayerDisplayNameProvider getPlayerDisplayNameProvider() {
        return providers.getPlayerDisplayNameProvider();
    }

    @Override
    public void setPlayerDisplayNameProvider(PlayerDisplayNameProvider playerNameProvider) {
        providers.setPlayerDisplayNameProvider(playerNameProvider);
    }

}
