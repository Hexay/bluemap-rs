package bluemaprs.paper.core;

/** The core cannot be started on this host at all; retrying will not help. The message is for the server admin. */
public final class CoreUnavailableException extends Exception {

    static final String OVERRIDE_HINT = "Set the BLUEMAP_CORE system property (-DBLUEMAP_CORE=/path/to/bluemap) or "
            + "environment variable to a BlueMap core binary built for this platform, or run BlueMap as the "
            + "standalone CLI next to the server.";

    CoreUnavailableException(String message) {
        super(message + " " + OVERRIDE_HINT);
    }

    CoreUnavailableException(String message, Throwable cause) {
        super(message + " " + OVERRIDE_HINT, cause);
    }

}
