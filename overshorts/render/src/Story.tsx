// 프리셋 고르기: 원문 사진·영상이 있으면 프레임형, 없으면 썰 카드형 (spec.preset 은 Python 이 정함).
import React from 'react';
import {CardStory} from './CardStory';
import {FrameStory} from './FrameStory';

export const Story: React.FC<Record<string, unknown>> = (props) =>
  (props?.preset === 'frame' ? <FrameStory {...props} /> : <CardStory {...props} />);
