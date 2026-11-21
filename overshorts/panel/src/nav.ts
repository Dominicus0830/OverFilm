/** 작업 공간 안에서 옮겨 가기: 다른 화면이 아니라 패널·창을 연다. */
export type Place = 'style' | 'settings' | 'voices' | 'close';
export type Go = (where: Place) => void;
