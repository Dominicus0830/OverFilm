import React from 'react';
import {Composition} from 'remotion';
import {CardStory} from './CardStory';
import {EMPTY_SPEC, sanitize} from './spec';
import {Story} from './Story';

const meta = ({props}: {props: Record<string, unknown>}) => {
  const s = sanitize(props);
  return {width: s.width, height: s.height, fps: s.fps,
    durationInFrames: Math.max(1, Math.round(s.duration * s.fps))};
};

export const Root: React.FC = () => (
  <>
  <Composition
    id="Story"
    component={Story}
    defaultProps={EMPTY_SPEC as unknown as Record<string, unknown>}
    width={1080}
    height={1920}
    fps={60}
    durationInFrames={Math.round(EMPTY_SPEC.duration * 60)}
    calculateMetadata={meta}
  />
  <Composition
    id="CardStory"
    component={CardStory}
    defaultProps={EMPTY_SPEC as unknown as Record<string, unknown>}
    width={1080}
    height={1920}
    fps={60}
    durationInFrames={Math.round(EMPTY_SPEC.duration * 60)}
    calculateMetadata={({props}) => {
      const s = sanitize(props);
      return {width: s.width, height: s.height, fps: s.fps,
        durationInFrames: Math.max(1, Math.round(s.duration * s.fps))};
    }}
  />
  </>
);
