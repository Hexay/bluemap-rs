package bluemaprs.shim.skins;

import de.bluecolored.bluemap.api.plugin.PlayerIconFactory;
import de.bluecolored.bluemap.core.logger.Logger;

import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.util.UUID;

/** Upstream's {@code common.plugin.skins.DefaultPlayerIconFactory}. */
public class DefaultPlayerIconFactory implements PlayerIconFactory {

    @Override
    public BufferedImage apply(UUID uuid, BufferedImage in) {
        BufferedImage head;

        BufferedImage layer1 = in.getSubimage(8, 8, 8, 8);
        BufferedImage layer2 = in.getSubimage(40, 8, 8, 8);

        try {
            head = new BufferedImage(48, 48, BufferedImage.TYPE_INT_ARGB);
            Graphics2D g = head.createGraphics();
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
            g.drawImage(layer1, 4, 4, 40, 40, null);
            g.drawImage(layer2, 0, 0, 48, 48, null);
        } catch (Throwable t) { // headless servers may fail to load Graphics2D
            Logger.global.noFloodWarning("headless-graphics-fail",
                    "Could not access Graphics2D to render player-skin texture. Try adding '-Djava.awt.headless=true' to your startup flags or ignore this warning.");
            head = new BufferedImage(8, 8, in.getType());
            layer1.copyData(head.getRaster());
        }

        return head;
    }

}
