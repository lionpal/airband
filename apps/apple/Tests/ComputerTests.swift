// Each picture a computer can be drawn with is a symbol this system has.
import UIKit
import XCTest
@testable import Airband

final class ComputerTests: XCTestCase {
    func testEveryPictureIsASymbolThatExists() {
        for picture in RadioModel.computerPictures {
            XCTAssertNotNil(UIImage(systemName: RadioModel.symbol(picture)), "\(picture)")
        }
        XCTAssertEqual(Set(RadioModel.computerPictures.map(RadioModel.symbol)).count, RadioModel.computerPictures.count)
        XCTAssertEqual(RadioModel.symbol(.plain), "display")
    }
}
