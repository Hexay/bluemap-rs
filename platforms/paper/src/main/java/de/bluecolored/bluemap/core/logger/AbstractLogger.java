package de.bluecolored.bluemap.core.logger;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.TimeUnit;

public abstract class AbstractLogger extends Logger {

    private static final long NO_FLOOD_TTL_NANOS = TimeUnit.MINUTES.toNanos(10);
    private static final int NO_FLOOD_MAX = 10000;

    private final Map<String, Long> noFlood = new LinkedHashMap<>() {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, Long> eldest) {
            return size() > NO_FLOOD_MAX;
        }
    };

    @Override
    public void noFloodError(String key, String message, Throwable throwable) {
        if (check(key)) logError(message, throwable);
    }

    @Override
    public void noFloodWarning(String key, String message) {
        if (check(key)) logWarning(message);
    }

    @Override
    public void noFloodInfo(String key, String message) {
        if (check(key)) logInfo(message);
    }

    @Override
    public void noFloodDebug(String key, String message) {
        if (check(key)) logDebug(message);
    }

    @Override
    public synchronized void clearNoFloodLog() {
        noFlood.clear();
    }

    @Override
    public synchronized void removeNoFloodKey(String key) {
        noFlood.remove(key);
    }

    private synchronized boolean check(String key) {
        long now = System.nanoTime();
        Long last = noFlood.get(key);
        if (last != null && now - last < NO_FLOOD_TTL_NANOS) return false;
        noFlood.remove(key);
        noFlood.put(key, now);
        return true;
    }

}
