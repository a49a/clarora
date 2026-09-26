import { Fragment, createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import { StyleSheet, View } from 'react-native';

// 页面级浮层宿主:听力页根层放置 PopoverRoot,设置面板/遮罩由 usePopoverPortal
// 挂载到这一层,避免绝对定位面板被标题栏父容器裁剪,并让遮罩与面板按可见
// 区域(宿主坐标系)定位。
export type PortalGeometry = { x: number; y: number; width: number; height: number };

type PortalApi = {
  /** 在页面层挂载元素;相同 id 覆盖 */
  mount: (id: string, element: ReactNode) => void;
  /** 移除挂载元素 */
  unmount: (id: string) => void;
  /** 宿主层在窗口坐标系中的位置与尺寸(与 measureInWindow 同一坐标系) */
  measureLayer: () => Promise<PortalGeometry>;
};

const PortalContext = createContext<PortalApi | null>(null);

export function usePopoverPortal(): PortalApi | null {
  return useContext(PortalContext);
}

export function PopoverRoot({ children }: { children: ReactNode }) {
  const [layers, setLayers] = useState<ReadonlyMap<string, ReactNode>>(new Map());
  const [layerRef, setLayerRef] = useState<View | null>(null);

  const mount = useCallback((id: string, element: ReactNode) => {
    setLayers(previous => {
      const next = new Map(previous);
      next.set(id, element);
      return next;
    });
  }, []);

  const unmount = useCallback((id: string) => {
    setLayers(previous => {
      if (!previous.has(id)) return previous;
      const next = new Map(previous);
      next.delete(id);
      return next;
    });
  }, []);

  const measureLayer = useCallback(() => {
    return new Promise<PortalGeometry>(resolve => {
      if (!layerRef) {
        resolve({ x: 0, y: 0, width: 0, height: 0 });
        return;
      }
      layerRef.measureInWindow((x, y, width, height) => resolve({ x, y, width, height }));
    });
  }, [layerRef]);

  const api = useMemo(() => ({ mount, unmount, measureLayer }), [mount, unmount, measureLayer]);

  return (
    <PortalContext.Provider value={api}>
      <View style={{ flex: 1 }}>
        {children}
        <View
          ref={setLayerRef}
          pointerEvents="none"
          collapsable={false}
          style={StyleSheet.absoluteFill}
        />
        {layers.size > 0 && <View style={[StyleSheet.absoluteFill, { zIndex: 40 }]}>
          {[...layers.entries()].map(([id, element]) => <Fragment key={id}>{element}</Fragment>)}
        </View>}
      </View>
    </PortalContext.Provider>
  );
}
