package bluemaprs.shim;

import de.bluecolored.bluemap.core.logger.AbstractLogger;
import org.slf4j.Logger;

/** Routes upstream's {@code Logger.global} facade (used by addons and the API impls) to the platform's SLF4J logger. */
public final class Slf4jLogger extends AbstractLogger {

    private final Logger out;

    public Slf4jLogger(Logger out) {
        this.out = out;
    }

    @Override
    public void logError(String message, Throwable throwable) {
        out.error(message, throwable);
    }

    @Override
    public void logWarning(String message) {
        out.warn(message);
    }

    @Override
    public void logInfo(String message) {
        out.info(message);
    }

    @Override
    public void logDebug(String message) {
        out.debug(message);
    }

    @Override
    public void noFloodDebug(String message) {
        if (out.isDebugEnabled()) super.noFloodDebug(message);
    }

    @Override
    public void noFloodDebug(String key, String message) {
        if (out.isDebugEnabled()) super.noFloodDebug(key, message);
    }

    @Override
    public void close() {}

}
