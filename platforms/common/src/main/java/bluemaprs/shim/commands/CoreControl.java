package bluemaprs.shim.commands;

/** What {@code /bluemap} needs from the core supervisor while the core is down. */
public interface CoreControl {

    /** Whether a respawn is already scheduled (false after giving up or while stopping). */
    boolean willRestart();

    /** Starts the core now, resetting the crash counter. */
    void restartNow();

}
