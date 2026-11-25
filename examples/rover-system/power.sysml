package Power {
    import ScalarValues::*;

    attribute def PowerFlow;

    port def PowerPort {
        out item electricalPower : PowerFlow;
    }

    interface def PowerInterface {
        end supply : PowerPort;
        end consumer : ~PowerPort;
    }

    part def BatteryPack {
        attribute capacityKWh : Real = 4.5;
        port powerOut : PowerPort;
    }

    part def ElectricMotor {
        attribute maxPowerKW : Real = 12.0;
        port powerIn : ~PowerPort;
    }

    part def HighTorqueMotor :> ElectricMotor {
        attribute peakTorqueNm : Real = 280.0;
    }

    part def Powertrain {
        part battery : BatteryPack [1];
        part motor : ElectricMotor [1];

        interface powerBus : PowerInterface
            connect battery.powerOut to motor.powerIn;
    }
}
