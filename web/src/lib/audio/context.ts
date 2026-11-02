/** The capture's AudioContext. Its own file: join() makes one inside the click before the capture code has loaded. */
export const captureContext = (): AudioContext => new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' });
