import Foundation

/// Any JSON value: command arguments, opaque payload parts (`ui`, `dsp`),
/// and events before they are typed.
public enum JSON: Sendable, Equatable, Hashable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSON])
    case object([String: JSON])

    public subscript(key: String) -> JSON {
        if case .object(let o) = self { return o[key] ?? .null }
        return .null
    }

    public subscript(index: Int) -> JSON {
        if case .array(let a) = self, a.indices.contains(index) { return a[index] }
        return .null
    }

    public var isNull: Bool { self == .null }
    public var string: String? { if case .string(let s) = self { return s }; return nil }
    public var bool: Bool? { if case .bool(let b) = self { return b }; return nil }
    public var double: Double? { if case .number(let n) = self { return n }; return nil }
    public var int: Int? { double.map { Int($0) } }
    public var array: [JSON]? { if case .array(let a) = self { return a }; return nil }
    public var object: [String: JSON]? { if case .object(let o) = self { return o }; return nil }

    /// Wraps any encodable value (`nil` → `.null`).
    public init<T: Encodable>(encoding value: T) throws {
        self = try JSON.parse(Wire.encoder.encode(value))
    }

    /// Parses JSON text (fragments allowed).
    public static func parse(_ data: Data) throws -> JSON {
        try JSONDecoder().decode(JSON.self, from: data)
    }

    public static func parse(_ text: String) throws -> JSON {
        try parse(Data(text.utf8))
    }

    /// Compact JSON text with sorted keys.
    public func encoded() -> String {
        let data = (try? Wire.encoder.encode(self)) ?? Data("null".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    /// Decodes into a model (snake_case and camelCase keys both map to
    /// camelCase properties, see `Wire`).
    public func decode<T: Decodable>(_: T.Type = T.self) throws -> T {
        try Wire.decoder.decode(T.self, from: Data(encoded().utf8))
    }
}

extension JSON: Codable {
    private struct AnyKey: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    public init(from decoder: Decoder) throws {
        if let c = try? decoder.container(keyedBy: AnyKey.self) {
            var o: [String: JSON] = [:]
            for k in c.allKeys { o[k.stringValue] = try c.decode(JSON.self, forKey: k) }
            self = .object(o)
        } else if var c = try? decoder.unkeyedContainer() {
            var a: [JSON] = []
            while !c.isAtEnd { a.append(try c.decode(JSON.self)) }
            self = .array(a)
        } else {
            let c = try decoder.singleValueContainer()
            if c.decodeNil() { self = .null }
            else if let b = try? c.decode(Bool.self) { self = .bool(b) }
            else if let n = try? c.decode(Double.self) { self = .number(n) }
            else { self = .string(try c.decode(String.self)) }
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .null:
            var c = encoder.singleValueContainer(); try c.encodeNil()
        case .bool(let b):
            var c = encoder.singleValueContainer(); try c.encode(b)
        case .number(let n):
            var c = encoder.singleValueContainer()
            if n.rounded() == n, abs(n) < 9.0e15 { try c.encode(Int64(n)) } else { try c.encode(n) }
        case .string(let s):
            var c = encoder.singleValueContainer(); try c.encode(s)
        case .array(let a):
            var c = encoder.unkeyedContainer()
            for v in a { try c.encode(v) }
        case .object(let o):
            var c = encoder.container(keyedBy: AnyKey.self)
            for (k, v) in o { try c.encode(v, forKey: AnyKey(stringValue: k)) }
        }
    }
}

extension JSON: ExpressibleByNilLiteral, ExpressibleByBooleanLiteral, ExpressibleByIntegerLiteral,
    ExpressibleByFloatLiteral, ExpressibleByStringLiteral, ExpressibleByArrayLiteral,
    ExpressibleByDictionaryLiteral
{
    public init(nilLiteral: ()) { self = .null }
    public init(booleanLiteral v: Bool) { self = .bool(v) }
    public init(integerLiteral v: Int) { self = .number(Double(v)) }
    public init(floatLiteral v: Double) { self = .number(v) }
    public init(stringLiteral v: String) { self = .string(v) }
    public init(arrayLiteral elements: JSON...) { self = .array(elements) }
    public init(dictionaryLiteral elements: (String, JSON)...) {
        self = .object(Dictionary(elements, uniquingKeysWith: { _, b in b }))
    }
}

public extension JSON {
    init(_ v: String?) { self = v.map(JSON.string) ?? .null }
    init(_ v: Bool?) { self = v.map(JSON.bool) ?? .null }
    init(_ v: Int?) { self = v.map { .number(Double($0)) } ?? .null }
    init(_ v: Double?) { self = v.map(JSON.number) ?? .null }
    init(_ v: [String]) { self = .array(v.map(JSON.string)) }
}

/// The wire conventions. Rust serialises the ytm models with snake_case
/// keys and everything else with camelCase; `convertFromSnakeCase` maps
/// both onto camelCase Swift properties (keys without `_` stay as they are).
/// Encoding is plain (camelCase); the few snake_case inputs (ytm models
/// sent back to `library_like`) use `Wire.snakeEncoder`.
public enum Wire {
    public static var decoder: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }

    public static var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return e
    }

    public static var snakeEncoder: JSONEncoder {
        let e = encoder
        e.keyEncodingStrategy = .convertToSnakeCase
        return e
    }
}
