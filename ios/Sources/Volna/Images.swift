#if os(iOS)
import SwiftUI
import UIKit
import VolnaKit

/// Covers through the core (`volna://img/`): its disk cache and its network,
/// with a memory cache on top.
@MainActor
final class ImageStore {
    static let shared = ImageStore()
    private let cache = NSCache<NSString, UIImage>()
    private var inflight: [String: Task<UIImage?, Never>] = [:]

    init() {
        cache.countLimit = 300
    }

    func image(core: Core, url: String, size: Int) async -> UIImage? {
        let uri = imageURI(url, size: size)
        if let hit = cache.object(forKey: uri as NSString) { return hit }
        if let t = inflight[uri] { return await t.value }
        let t = Task<UIImage?, Never> {
            let r = await core.proto(uri)
            guard r.status == 200 else { return nil }
            return UIImage(data: r.body)
        }
        inflight[uri] = t
        let img = await t.value
        inflight[uri] = nil
        if let img { cache.setObject(img, forKey: uri as NSString) }
        return img
    }
}

/// A square (or round) cover with a quiet placeholder.
struct Cover: View {
    @Environment(AppModel.self) private var app
    let url: String?
    var size: CGFloat = 48
    var corner: CGFloat = 8
    var round = false
    @Environment(\.displayScale) private var scale
    @State private var image: UIImage?

    var body: some View {
        ZStack {
            Rectangle().fill(.quaternary)
            if let image {
                Image(uiImage: image).resizable().scaledToFill().transition(.opacity)
            } else {
                Image(systemName: "music.note").foregroundStyle(.tertiary).font(.system(size: size * 0.3))
            }
        }
        .frame(width: size, height: size)
        .clipShape(round ? AnyShape(Circle()) : AnyShape(RoundedRectangle(cornerRadius: corner, style: .continuous)))
        .task(id: url) {
            image = nil
            guard let url, let core = app.core else { return }
            let px = Int(size * scale)
            let img = await ImageStore.shared.image(core: core, url: url, size: min(1200, max(120, px)))
            withAnimation(.easeOut(duration: 0.2)) { image = img }
        }
    }
}

/// A cover that fills its width (player, headers).
struct LargeCover: View {
    @Environment(AppModel.self) private var app
    let url: String?
    var corner: CGFloat = 14
    @State private var image: UIImage?

    var body: some View {
        GeometryReader { g in
            ZStack {
                RoundedRectangle(cornerRadius: corner, style: .continuous).fill(.quaternary)
                if let image {
                    Image(uiImage: image).resizable().scaledToFill()
                        .frame(width: g.size.width, height: g.size.width)
                        .transition(.opacity)
                } else {
                    Image(systemName: "music.note").font(.system(size: 64)).foregroundStyle(.tertiary)
                }
            }
            .frame(width: g.size.width, height: g.size.width)
            .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
        }
        .aspectRatio(1, contentMode: .fit)
        .task(id: url) {
            guard let url, let core = app.core else { image = nil; return }
            let img = await ImageStore.shared.image(core: core, url: url, size: 1000)
            withAnimation(.easeInOut(duration: 0.35)) { image = img }
        }
    }
}

extension Color {
    init?(hex: String?) {
        guard let hex, let (r, g, b) = Palette.rgb(hex) else { return nil }
        self.init(red: r, green: g, blue: b)
    }
}
#endif
