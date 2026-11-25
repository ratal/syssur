package RoverSystem {
    import Power::*;
    import ChassisPkg::*;

    requirement def <'REQ-01'> PowerBudgetReq {
        doc /* Rover powertrain must deliver at least 10 kW continuous power. */
        subject pt : Powertrain [1];
        require constraint { pt.motor.maxPowerKW >= 10.0 }
    }

    requirement def <'REQ-02'> MobilityReq {
        doc /* Rover must support 4-wheel drive with steerable front axle. */
    }

    part def Rover {
        part powertrain : Powertrain [1];
        part chassis : Chassis [1];

        connect powertrain.motor to chassis.frontLeftWheel;
        connect powertrain.motor to chassis.frontRightWheel;
    }

    part def HeavyDutyRover :> Rover {
        part :>> powertrain {
            part :>> motor : HighTorqueMotor [1];
        }
    }

    part nominalRover : Rover [1];
    part heavyRover : HeavyDutyRover [1];

    requirement powerBudgetReq : PowerBudgetReq;
    requirement mobilityReq : MobilityReq;

    satisfy powerBudgetReq by nominalRover.powertrain;
    satisfy mobilityReq by nominalRover.chassis;
}
