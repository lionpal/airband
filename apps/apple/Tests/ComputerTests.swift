// Each shape the core gives a computer is drawn with a symbol this system has.
import UIKit
import XCTest
@testable import Airband

final class ComputerTests: XCTestCase {
    func testEveryShapeIsDrawnWithASymbolThatExists() {
        let shapes: [MachineShape] = [.laptop, .mini, .studio, .imac, .pro, .unknown]
        for shape in shapes {
            XCTAssertNotNil(UIImage(systemName: RadioModel.symbol(shape)), "\(shape)")
        }
        XCTAssertEqual(Set(shapes.map(RadioModel.symbol)).count, shapes.count)
        XCTAssertEqual(RadioModel.symbol(.unknown), "display")
    }
}
