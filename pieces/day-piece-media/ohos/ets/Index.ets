// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-media — the HarmonyOS half of the piece (docs/extending.md, docs/media.md).
//
// The ArkUI C node API has no video node kind, so the player is declared here in ArkTS: a `Video`
// component for pictures (built into a BuilderNode and handed to Day as a FrameNode the native
// tree mounts like any other node), or an `AVPlayer` from @kit.MediaKit for sound only, behind an
// empty Column of no size. `day build` stages this file into the app's hvigor project and
// generates the aggregator that registers `dayPiece` with the shim.
//
// Commands arrive as the piece's own vocabulary ('load' | 'play' | 'pause' | 'stop' | 'volume').
// Playback state goes back through `pieceEvent` with the piece's report codes as `num` (0 idle,
// 1 loading, 2 playing, 3 paused, 4 ended, 5 error), which reaches the Rust front-end as an
// `Event::Custom` — the same channel the Android renderer uses.

import { BuilderNode, FrameNode, UIContext } from '@kit.ArkUI';
import { media } from '@kit.MediaKit';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';
import { DayPieceModule } from '../DayPiece';

const IDLE: number = 0;
const LOADING: number = 1;
const PLAYING: number = 2;
const PAUSED: number = 3;
const ENDED: number = 4;
const ERROR: number = 5;

function report(id: number, code: number, text: string = ''): void {
  nativeEntry.pieceEvent(id, text, code);
}

// What the props string carries, 0x1F-separated in this order (lib-arkui.rs composes it).
class DayMediaParams {
  id: number = 0;
  url: string = '';
  autoplay: boolean = true;
  looping: boolean = false;
  muted: boolean = false;
  controls: boolean = true;
  audioOnly: boolean = false;
  volume: number = 1.0;
  controller: VideoController = new VideoController();
}

function parseParams(id: number, props: string): DayMediaParams {
  const p: DayMediaParams = new DayMediaParams();
  p.id = id;
  const parts: string[] = props.split('\u001F');
  p.url = parts.length > 0 ? parts[0] : '';
  p.autoplay = parts.length > 1 ? parts[1] === '1' : true;
  p.looping = parts.length > 2 ? parts[2] === '1' : false;
  p.muted = parts.length > 3 ? parts[3] === '1' : false;
  p.controls = parts.length > 4 ? parts[4] === '1' : true;
  p.audioOnly = parts.length > 5 ? parts[5] === '1' : false;
  const v: number = parts.length > 6 ? Number(parts[6]) : 1.0;
  p.volume = Number.isNaN(v) ? 1.0 : v;
  return p;
}

// One live player, either shape. Held until Day disposes the node.
class DayMediaInstance {
  node: BuilderNode<[DayMediaParams]>;
  params: DayMediaParams;
  // The sound-only engine; a Video component drives its own.
  player: media.AVPlayer | null = null;
  // Whether the app asked to play before the source was ready.
  playWhenReady: boolean = false;

  constructor(node: BuilderNode<[DayMediaParams]>, params: DayMediaParams) {
    this.node = node;
    this.params = params;
  }
}

// The picture player. NO percentage sizing: a BuilderNode is built detached, so '100%' resolves
// against the whole window. Day owns layout and sets this node's position + size through the
// ArkUI C API like every other leaf, which is what the piece's `fill_measure` asks for.
@Builder
function dayVideoBuilder(params: DayMediaParams) {
  Video({ src: params.url, controller: params.controller })
    .autoPlay(params.autoplay)
    .loop(params.looping)
    .muted(params.muted)
    .controls(params.controls)
    .onPrepared(() => { report(params.id, params.autoplay ? PLAYING : PAUSED); })
    .onStart(() => { report(params.id, PLAYING); })
    .onPause(() => { report(params.id, PAUSED); })
    .onFinish(() => { if (!params.looping) { report(params.id, ENDED); } })
    .onStop(() => { report(params.id, IDLE); })
    .onError(() => { report(params.id, ERROR, 'playback failed'); })
}

// The sound-only leaf: nothing to draw, so an empty container Day measures at zero.
@Builder
function dayAudioBuilder(params: DayMediaParams) {
  Column()
    .width(0)
    .height(0)
}

const instances: Map<number, DayMediaInstance> = new Map();

// Build the AVPlayer for `url` and start it when it reports ready. The player walks a state
// machine (idle → initialized → prepared → playing …); each step is a `stateChange` event, which
// is also where the piece's own report comes from.
function loadAudio(live: DayMediaInstance, url: string, playWhenReady: boolean): void {
  releaseAudio(live);
  live.playWhenReady = playWhenReady;
  report(live.params.id, LOADING);
  media.createAVPlayer().then((player: media.AVPlayer) => {
    if (instances.get(live.params.id) !== live) {
      player.release();
      return;
    }
    live.player = player;
    player.on('stateChange', (state: string) => {
      if (live.player !== player) {
        return;
      }
      if (state === 'initialized') {
        player.loop = live.params.looping;
        player.prepare().catch((err: BusinessError) => {
          report(live.params.id, ERROR, String(err.message));
        });
      } else if (state === 'prepared') {
        player.setVolume(live.params.muted ? 0 : live.params.volume);
        if (live.playWhenReady) {
          player.play();
        } else {
          report(live.params.id, PAUSED);
        }
      } else if (state === 'playing') {
        report(live.params.id, PLAYING);
      } else if (state === 'paused') {
        report(live.params.id, PAUSED);
      } else if (state === 'completed') {
        if (!live.params.looping) {
          report(live.params.id, ENDED);
        }
      } else if (state === 'stopped' || state === 'idle') {
        report(live.params.id, IDLE);
      } else if (state === 'error') {
        report(live.params.id, ERROR, 'playback failed');
      }
    });
    player.on('error', (err: BusinessError) => {
      if (live.player === player) {
        report(live.params.id, ERROR, String(err.message));
      }
    });
    // A stalled network shows up as buffering updates rather than a state change.
    player.on('bufferingUpdate', (infoType: media.BufferingInfoType) => {
      if (live.player !== player) {
        return;
      }
      if (infoType === media.BufferingInfoType.BUFFERING_START) {
        report(live.params.id, LOADING);
      } else if (infoType === media.BufferingInfoType.BUFFERING_END) {
        report(live.params.id, player.state === 'playing' ? PLAYING : PAUSED);
      }
    });
    // Setting the url moves the player to `initialized`, which the handler above continues.
    player.url = url;
  }).catch((err: BusinessError) => {
    report(live.params.id, ERROR, String(err.message));
  });
}

function releaseAudio(live: DayMediaInstance): void {
  const old: media.AVPlayer | null = live.player;
  live.player = null;
  if (old !== null) {
    try {
      old.release();
    } catch (err) {
      console.warn(`day-piece-media: release failed: ${err}`);
    }
  }
}

function make(ui: UIContext, id: number, props: string): FrameNode | undefined {
  const params: DayMediaParams = parseParams(id, props);
  const node: BuilderNode<[DayMediaParams]> = new BuilderNode(ui);
  if (params.audioOnly) {
    node.build(wrapBuilder<[DayMediaParams]>(dayAudioBuilder), params);
  } else {
    node.build(wrapBuilder<[DayMediaParams]>(dayVideoBuilder), params);
  }
  const frame: FrameNode | null = node.getFrameNode();
  if (frame === null) {
    return undefined;
  }
  const live: DayMediaInstance = new DayMediaInstance(node, params);
  instances.set(id, live);
  if (params.audioOnly && params.url !== '') {
    loadAudio(live, params.url, params.autoplay);
  }
  return frame;
}

function update(id: number, cmd: string, arg: string): void {
  const live: DayMediaInstance | undefined = instances.get(id);
  if (live === undefined) {
    return;
  }
  if (live.params.audioOnly) {
    const player: media.AVPlayer | null = live.player;
    try {
      if (cmd === 'load') {
        loadAudio(live, arg, true);
      } else if (cmd === 'play') {
        live.playWhenReady = true;
        if (player !== null && (player.state === 'prepared' || player.state === 'paused'
          || player.state === 'completed' || player.state === 'stopped')) {
          if (player.state === 'stopped') {
            player.prepare();
          } else {
            player.play();
          }
        }
      } else if (cmd === 'pause') {
        live.playWhenReady = false;
        if (player !== null && player.state === 'playing') {
          player.pause();
        }
      } else if (cmd === 'stop') {
        // Dropping the player is what lets a live stream's connection go.
        releaseAudio(live);
        report(id, IDLE);
      } else if (cmd === 'volume') {
        const v: number = Number(arg);
        if (!Number.isNaN(v)) {
          live.params.volume = v;
          if (player !== null && !live.params.muted) {
            player.setVolume(v);
          }
        }
      }
    } catch (err) {
      console.error(`day-piece-media: ${cmd} failed: ${err}`);
    }
    return;
  }
  // The picture player: driven through its VideoController. A `load` swaps the source by
  // rebuilding the component (the Video's src is fixed at build), which is also what a `stop`
  // needs to let the connection go.
  try {
    if (cmd === 'load') {
      live.params.url = arg;
      live.params.autoplay = true;
      live.node.update(live.params);
      report(id, LOADING);
    } else if (cmd === 'play') {
      live.params.controller.start();
    } else if (cmd === 'pause') {
      live.params.controller.pause();
    } else if (cmd === 'stop') {
      live.params.controller.stop();
      report(id, IDLE);
    } else if (cmd === 'volume') {
      // The Video component exposes mute but no volume; a muted player stays muted.
      const v: number = Number(arg);
      if (!Number.isNaN(v)) {
        live.params.volume = v;
      }
    }
  } catch (err) {
    console.error(`day-piece-media: ${cmd} failed: ${err}`);
  }
}

function dispose(id: number): void {
  const live: DayMediaInstance | undefined = instances.get(id);
  if (live !== undefined) {
    instances.delete(id);
    releaseAudio(live);
    live.node.dispose();
  }
}

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.media',
  make: make,
  update: update,
  dispose: dispose
};
