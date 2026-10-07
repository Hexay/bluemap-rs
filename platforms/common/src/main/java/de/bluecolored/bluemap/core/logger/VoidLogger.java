package de.bluecolored.bluemap.core.logger;

public class VoidLogger extends Logger {

    @Override
    public void logError(String message, Throwable throwable) {}

    @Override
    public void logWarning(String message) {}

    @Override
    public void logInfo(String message) {}

    @Override
    public void logDebug(String message) {}

    @Override
    public void noFloodError(String key, String message, Throwable throwable) {}

    @Override
    public void noFloodWarning(String key, String message) {}

    @Override
    public void noFloodInfo(String key, String message) {}

    @Override
    public void noFloodDebug(String key, String message) {}

    @Override
    public void clearNoFloodLog() {}

    @Override
    public void removeNoFloodKey(String key) {}

}
